//! Sky Squid: mid-range pellets and a one-second invulnerable escape.
//!
//! Ink Shotgun is the best-documented ability in the whole roster -- the wiki
//! gives seven pellets at 1.725 damage each, 12.075 if every one lands -- so it
//! is the one place a kit's damage is exact rather than described.
//!
//! Stats verified: 6.0 damage, 10 armour points (40%), 150% knockback taken,
//! 0.25 regen, 3000 gems.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::entity_kind::EntityKind;

use crate::module::{
    ability::{self, Cast, Observable, splash_at},
    effect::{self, Affliction},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    projectile::{Flight, Payload, Visual, fire},
    visuals,
};

/// `[VERIFIED]` "Each pellet deals 1.725 damage, so a total damage of 12.075 if
/// all pellets were to hit their target."
pub const PELLETS: usize = 7;
pub const PELLET_DAMAGE: f32 = 1.725;

/// `[VERIFIED]`: "one second of flight, and nothing can touch you during it".
///
/// The window is exactly the flight, which is what makes the ability an escape
/// rather than a reposition: a Squid who uses it to cross a gap arrives with
/// nothing left, and one who uses it to eat a Creeper's Explosion has spent it
/// correctly.
pub const SUPER_SQUID_SECONDS: f32 = 1.0;

#[derive(Component)]
pub struct SkySquid;

impl Module for SkySquid {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::SkySquid");

        kit::define(world, "Sky Squid", KitStats {
            melee_damage: 6.0,
            armor: 10.0,
            knockback_taken: 1.50,
            regen: 0.25,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.squid.ambient",
            hurt: "minecraft:entity.squid.hurt",
            death: "minecraft:entity.squid.death",
        })
        .cost(3000)
        .skin(crate::kit_skin!("sky_squid"))
        .blurb("Seven pellets up close, and one second of being untouchable.")
        .mob("minecraft:squid")
        .ability(AbilitySpec {
            name: "Super Squid",
            sound: "minecraft:entity.squid.ambient",
            item: "minecraft:iron_sword",
            description: "One second of flight, and nothing can touch you during it.",
            cooldown: 9.0,
            proves: &[Observable::LaunchesCaster, Observable::ShieldsCaster],
            activate: super_squid,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Ink Shotgun",
            sound: "minecraft:entity.squid.squirt",
            item: "minecraft:iron_axe",
            description: "Seven ink sacs at once. All seven is 12 damage and almost never happens.",
            cooldown: 5.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: ink_shotgun,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Fish Flurry",
            sound: "minecraft:entity.cod.flop",
            item: "minecraft:iron_shovel",
            description: "Fish erupt from the ground for four seconds. Hard to walk out of.",
            // `[VERIFIED]`: "a ridiculous 16 seconds cooldown to balance it".
            cooldown: 16.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: fish_flurry,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Storm Squid",
            sound: "minecraft:entity.lightning_bolt.thunder",
            item: "minecraft:nether_star",
            description: "Fly, and call lightning down once a second for twenty seconds.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
                Observable::Sustains,
            ],
            activate: storm_squid,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// `[VERIFIED]` one second of flight and one second of being untouchable.
/// `[APPROXIMATED]` impulse.
///
/// The shield is an effect rather than a flag on the player, so the window ends
/// on its own even if the Squid dies, disconnects or changes kit inside it.
fn super_squid(cast: &Cast<'_>) {
    cast.server.add_velocity(
        cast.player,
        cast.facing.0.normalize_or_zero() * 1.1 + Vec3::Y * 0.9,
    );
    // One impulse and not a mode, so the wake is drawn once, where the dash
    // starts. The second it lasts is the shield's, and the shield has its own
    // look.
    cast.server.particles(visuals::wake(cast.position.0));
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::shield(SUPER_SQUID_SECONDS),
    );
}

fn ink_shotgun(cast: &Cast<'_>) {
    const SPREAD: f32 = 0.16;
    let forward = cast.facing.0.normalize_or_zero();
    let side = Vec3::new(-forward.z, 0.0, forward.x);
    for index in 0..PELLETS {
        let offset = (index as f32 - (PELLETS as f32 - 1.0) / 2.0) * SPREAD;
        let direction = (forward + side * offset).normalize_or_zero();
        fire(
            cast.world,
            cast.caster,
            // `[APPROXIMATED]`: an ink pellet has no entity; a snowball is the
            // closest small thrown blob that always renders.
            Visual(EntityKind::Snowball),
            Flight {
                position: cast.position.0,
                velocity: direction * 26.0,
                gravity: 10.0,
                // Short range on purpose: the wiki's complaint about the kit is
                // that the pellets are useless far out.
                seconds_left: 0.7,
                radius: 0.6,
            },
            Payload::new(PELLET_DAMAGE, 0.6),
        );
    }
    // Once for the press, not once per pellet: seven ink clouds down one cone
    // is a smokescreen, and the one fact a victim needs is where the cone is.
    cast.server
        .particles(visuals::ink(cast.position.0, forward));
}

/// `[VERIFIED]` 5x5x5 area, 2 damage per fish, four seconds. Modelled as one
/// splash rather than as individual dropped items, because the items would need
/// the host's entity system and the damage is the part that matters.
fn fish_flurry(cast: &Cast<'_>) {
    let at = cast.position.0 + cast.facing.0.normalize_or_zero() * 4.0;
    splash_at(cast, at, 2.5, 2.0, 0.7);
    cast.server.particles(visuals::blast(at));
}

/// `[VERIFIED]` "call lightning down once a second"; the twenty seconds is
/// [`crate::module::ability::ULTIMATE_SECONDS`], and the per-bolt damage is
/// `[APPROXIMATED]`.
///
/// The ability is the duration. It used to be one strike, because the ability
/// layer had no way to say "and again, nineteen more times"; it now says
/// exactly that and the strike itself is unchanged.
fn storm_squid(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, STORM_INTERVAL, storm_bolt),
    );
}

/// `[VERIFIED]` one second.
const STORM_INTERVAL: f32 = 1.0;

/// One bolt on each other player, wherever they are standing, and a beat of
/// flight for the squid.
fn storm_bolt(cast: &Cast<'_>) {
    use crate::module::{ability::splash_from, player::Position};

    // "Fly" -- a small lift every beat, which is the closest the seam gets to
    // a flight mode and is enough to keep a squid off the floor for the
    // duration.
    cast.server.add_velocity(cast.player, Vec3::Y * STORM_LIFT);

    let caster = cast.caster.id();
    let mut targets = Vec::new();
    cast.world
        .query::<&Position>()
        .with(crate::module::player::Player::id())
        .build()
        .each_entity(|entity, position| {
            if entity.id() != caster {
                targets.push(position.0);
            }
        });
    for at in targets {
        // The bolt lands on the victim, so the launch has to be measured from
        // the squid: away from the point you are standing on is not a direction.
        splash_from(cast, cast.position.0, at, 2.0, STORM_DAMAGE, 1.2);
        cast.server.particles(visuals::blast(at));
    }
}

/// `[APPROXIMATED]`. Per bolt, and there are twenty of them, so this is
/// deliberately a fraction of what the single-strike version dealt: the old
/// number applied once a second for twenty seconds would be 120 damage.
const STORM_DAMAGE: f32 = 2.0;

/// `[APPROXIMATED]`. Enough lift per beat to hold a squid up without launching
/// them off the map, which a bolt's own knockback would do if this were larger.
const STORM_LIFT: f32 = 0.35;
