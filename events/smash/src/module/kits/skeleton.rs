//! Skeleton: the ranged kit.
//!
//! The first kit in the game and the one you get if you pick nothing. Every
//! ability is a bow or a way to survive someone reaching the bow: Barrage is a
//! hold-and-release that stacks up to five arrows, Bone Explosion is the panic
//! button with the highest knockback multiplier of any starting ability in the
//! game, and Roped Arrow doubles as its recovery.
//!
//! Numbers from the `SMASH_KITS` spreadsheet dump.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::entity_kind::EntityKind;

use crate::module::{
    ability::{Cast, Observable, charge_steps, splash},
    effect::{self, Affliction},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    player::Position,
    projectile::{Flight, Impact, Payload, Visual, fire},
};

/// Flat, regardless of draw. `KitSkeleton.arrowDamage` overwrites the vanilla
/// charge-scaled roll.
pub const ARROW_DAMAGE: f32 = 6.0;

/// `AddKnockback("Knockback Arrow", 1.5)`.
pub const ARROW_KNOCKBACK: f32 = 1.5;

/// Barrage caps at five arrows.
pub const MAX_BARRAGE_ARROWS: u32 = 5;

#[derive(Component)]
pub struct Skeleton;

impl Module for Skeleton {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Skeleton");

        kit::define(world, "Skeleton", KitStats {
            melee_damage: 5.0,
            armor: 12.0,
            knockback_taken: 1.25,
            regen: 0.20,
            hunger_interval: 7.0,
            jump_power: 0.9,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.skeleton.ambient",
            hurt: "minecraft:entity.skeleton.hurt",
            death: "minecraft:entity.skeleton.death",
        })
        .skin(crate::kit_skin!("skeleton"))
        .blurb("Keep everyone at arm's length, then overcharge the bow and let go.")
        .mob("minecraft:skeleton")
        .ability(AbilitySpec {
            name: "Barrage",
            sound: "minecraft:entity.arrow.shoot",
            item: "minecraft:bow",
            description: "Hold to load arrows, up to five. Release to fire them all.",
            cooldown: 0.0,
            // 1000 ms to the first arrow, 300 ms per arrow after: five arrows
            // is 2.2 seconds of standing still.
            charge_time: Some(2.2),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: barrage,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Bone Explosion",
            sound: "minecraft:block.bone_block.break",
            item: "minecraft:iron_axe",
            description: "Scatter your bones. Little damage, enormous knockback.",
            cooldown: 10.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: bone_explosion,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Roped Arrow",
            sound: "minecraft:entity.fishing_bobber.throw",
            item: "minecraft:arrow",
            description: "Fire an arrow and be dragged after it. Your way back onto the map.",
            cooldown: 5.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: roped_arrow,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Arrow Storm",
            sound: "minecraft:item.crossbow.shoot",
            item: "minecraft:nether_star",
            description: "Eight seconds of firing without ever reloading.",
            cooldown: 8.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
            activate: arrow_storm,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

fn arrow(cast: &Cast<'_>, spread: f32) {
    let jitter = Vec3::new(spread, spread * 0.5, spread);
    fire(
        cast.world,
        cast.caster,
        Visual(EntityKind::Arrow),
        Flight {
            position: cast.position.0,
            velocity: (cast.facing.0.normalize_or_zero() + jitter) * 60.0,
            gravity: 20.0,
            seconds_left: 3.0,
            radius: 0.4,
        },
        Payload::new(ARROW_DAMAGE, ARROW_KNOCKBACK),
    );
}

/// One arrow per full fifth of charge, so a tap fires one and a full hold fires
/// five.
fn barrage(cast: &Cast<'_>) {
    let arrows = 1 + charge_steps(cast.charge, MAX_BARRAGE_ARROWS - 1);
    for index in 0..arrows.min(MAX_BARRAGE_ARROWS) {
        arrow(cast, index as f32 * 0.02);
    }
}

/// Damage 6 over radius 7 at a 2.5x knockback multiplier — the highest of any
/// starting ability, and why a Skeleton is not free to approach.
fn bone_explosion(cast: &Cast<'_>) {
    splash(cast, 7.0, 6.0, 2.5);
}

/// Fires an arrow and hauls the caster along behind it.
///
/// Mineplex's pull is `velocity(0.4 + mult * power, ...)` scaled by the arrow's
/// own speed at the moment of the hit. Without a real arrow to sample, the
/// impulse is taken from the launch direction, which matches at the ranges the
/// ability is actually used at.
fn roped_arrow(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Visual(EntityKind::Arrow),
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 48.0,
            gravity: 12.0,
            seconds_left: 2.0,
            radius: 0.4,
        },
        Payload::new(ARROW_DAMAGE, ARROW_KNOCKBACK).then(haul_shooter),
    );

    let pull = cast.facing.0.normalize_or_zero() * 1.4 + Vec3::Y * 0.6;
    cast.server.add_velocity(cast.player, pull);
}

fn haul_shooter(impact: &Impact<'_>) {
    let Some(shooter) = impact.shooter else {
        return;
    };
    let (Some(id), Some(from)) = (
        shooter.try_get::<&crate::server::PlayerId>(|p| *p),
        shooter.try_get::<&Position>(|p| p.0),
    ) else {
        return;
    };
    let pull = (impact.at - from).normalize_or_zero() * 1.2 + Vec3::Y * 0.6;
    impact
        .world
        .get::<&crate::server::ServerHandle>(|server| server.add_velocity(id, pull));
}

/// "Fire without ever reloading" is a duration, not a volley.
///
/// It used to be ten arrows in one frame, which is the opposite of never
/// reloading: it is one enormous reload. Eight seconds of a Barrage-sized
/// volley several times a second is the sentence the wiki actually writes, and
/// it is the same `arrow` the rest of the kit fires.
fn arrow_storm(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ARROW_STORM_SECONDS, ARROW_STORM_INTERVAL, arrow_volley),
    );
}

/// `[SHEET]` the ultimate's cooldown is 8 s, and Mineplex's ultimates run for
/// about as long as they take to come back.
const ARROW_STORM_SECONDS: f32 = 8.0;

/// `[APPROXIMATED]`. One arrow, three times a second.
///
/// One and not a Barrage-sized five: "without ever reloading" removes the draw,
/// it does not multiply the arrow. Five per beat is 90 damage a second, which
/// killed both scripted victims inside the first second and then read as the
/// ability having stopped -- the strongest possible version of the mode looking
/// exactly like a broken one.
const ARROW_STORM_INTERVAL: f32 = 0.3;

fn arrow_volley(cast: &Cast<'_>) {
    arrow(cast, 0.0);
}
