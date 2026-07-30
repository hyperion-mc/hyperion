//! Enderman: the movement kit.
//!
//! Two of its three abilities do no damage at all. Blink covers sixteen blocks
//! instantly on a seven-second cooldown, which is the best recovery in the
//! game, and the crouch-charged Teleport goes wherever you are looking as long
//! as nobody interrupts it. Block Toss is the reason people call the kit a
//! camper: a two-second cooldown on a 2.5x-knockback projectile.
//!
//! Numbers from the `SMASH_KITS` spreadsheet dump.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::entity_kind::EntityKind;

use crate::module::{
    ability::{self, Cast, Observable},
    effect::{self, Affliction},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    player::Position,
    projectile::{Flight, Payload, Visual, fire},
    visuals,
};

/// `Distance = 16`.
pub const BLINK_DISTANCE: f32 = 16.0;

/// `Damage = 8`, `Max Damage = 9`: charge buys one point.
pub const BLOCK_TOSS_MIN_DAMAGE: f32 = 8.0;
pub const BLOCK_TOSS_MAX_DAMAGE: f32 = 9.0;

#[derive(Component)]
pub struct Enderman;

impl Module for Enderman {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Enderman");

        kit::define(world, "Enderman", KitStats {
            melee_damage: 7.0,
            armor: 12.0,
            knockback_taken: 1.30,
            regen: 0.25,
            hunger_interval: 7.75,
            jump_power: 0.9,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.enderman.ambient",
            hurt: "minecraft:entity.enderman.hurt",
            death: "minecraft:entity.enderman.death",
        })
        .cost(4000)
        .skin(crate::kit_skin!("enderman"))
        .blurb("Throw the arena at people, then blink away before they reach you.")
        .mob("minecraft:enderman")
        .ability(AbilitySpec {
            name: "Block Toss",
            sound: "minecraft:block.stone.place",
            item: "minecraft:iron_sword",
            description: "Pick up a block and hurl it. Charge it for the extra point.",
            cooldown: 2.0,
            charge_time: Some(1.2),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: block_toss,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Blink",
            sound: "minecraft:entity.enderman.teleport",
            item: "minecraft:iron_axe",
            description: "Instantly cross sixteen blocks in the direction you are looking.",
            cooldown: 7.0,
            proves: &[Observable::TeleportsCaster],
            activate: blink,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Teleport",
            sound: "minecraft:entity.ender_pearl.throw",
            item: "minecraft:compass",
            description: "Charge, then go wherever you are looking. Getting hit cancels it.",
            cooldown: 5.0,
            charge_time: Some(1.0),
            proves: &[Observable::TeleportsCaster],
            activate: long_teleport,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Dragon Rider",
            sound: "minecraft:entity.ender_dragon.flap",
            item: "minecraft:nether_star",
            description: "Ride a dragon through everyone, for twenty seconds.",
            cooldown: 30.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
                Observable::Sustains,
            ],
            activate: dragon_rider,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// Charge buys a single point of damage, from 8 to 9. The reason to charge is
/// the throw speed, not the number.
fn block_toss(cast: &Cast<'_>) {
    let damage =
        (BLOCK_TOSS_MAX_DAMAGE - BLOCK_TOSS_MIN_DAMAGE).mul_add(cast.charge, BLOCK_TOSS_MIN_DAMAGE);
    // `min(1.4, 1.4 * elapsed / chargeTime)`.
    let speed = 1.4f32.min(1.4 * cast.charge).max(0.4) * 24.0;

    // At the caster's feet, where the block is ripped out of the floor. The
    // block is a visible falling-block entity from the moment it is thrown, so
    // the part that was never drawn is it coming loose.
    cast.server.particles(visuals::torn_earth(cast.position.0));

    fire(
        cast.world,
        cast.caster,
        Visual(EntityKind::FallingBlock),
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * speed,
            gravity: 8.0,
            seconds_left: 3.0,
            radius: 0.65,
        },
        Payload::new(damage, 2.5),
    );
}

fn blink(cast: &Cast<'_>) {
    teleport_to(
        cast,
        cast.position.0 + cast.facing.0.normalize_or_zero() * BLINK_DISTANCE,
    );
}

/// A hundred-block reach, but you have to stand still to charge it and any hit
/// cancels the charge — which is what [`crate::module::ability::Charging`]
/// being a component rather than a field makes cheap to express.
fn long_teleport(cast: &Cast<'_>) {
    const MAX_RANGE: f32 = 100.0;
    teleport_to(
        cast,
        cast.position.0 + cast.facing.0.normalize_or_zero() * (MAX_RANGE * cast.charge),
    );
}

/// `[APPROXIMATED]` throughout; the wiki names the ultimate and gives no
/// figures.
///
/// "Ride a dragon through everyone" is two verbs and the code did neither: the
/// Enderman never moved, so nothing was ridden and nobody was passed through.
/// Each beat now carries the rider forward and hits whatever is in front of
/// them, which is the sentence.
fn dragon_rider(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, RIDE_INTERVAL, dragon_pass),
    );
}

const RIDE_INTERVAL: f32 = 1.0;

/// Per pass, and there are twenty.
const RIDE_DAMAGE: f32 = 5.0;

fn dragon_pass(cast: &Cast<'_>) {
    let forward = cast.facing.0.normalize_or_zero();
    // The ride. Forward and a little up, so the dragon clears the lip of a
    // platform rather than stopping at it.
    cast.server
        .add_velocity(cast.player, forward * 1.6 + Vec3::Y * 0.3);
    // On each pass rather than at the cast: the ultimate is twenty seconds of
    // flight, and one puff at the start is nineteen seconds in which nobody
    // underneath can see what is above them.
    cast.server.particles(visuals::wingbeat(cast.position.0));
    crate::module::ability::splash_at(cast, cast.position.0 + forward * 8.0, 6.0, RIDE_DAMAGE, 4.0);
}

fn teleport_to(cast: &Cast<'_>, to: Vec3) {
    cast.caster.set(Position(to));
    cast.server.teleport(cast.player, to);
    cast.server.particles(visuals::teleport(to));
}
