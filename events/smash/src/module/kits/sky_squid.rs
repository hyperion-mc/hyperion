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

use crate::{
    module::{
        ability::{Cast, splash_at},
        kit::{self, AbilitySpec, KitStats},
        projectile::{Flight, Payload, fire},
    },
    server::Cue,
};

/// `[VERIFIED]` "Each pellet deals 1.725 damage, so a total damage of 12.075 if
/// all pellets were to hit their target."
pub const PELLETS: usize = 7;
pub const PELLET_DAMAGE: f32 = 1.725;

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
        .cost(3000)
        .blurb("Seven pellets up close, and one second of being untouchable.")
        .ability(AbilitySpec {
            name: "Super Squid",
            item: "minecraft:iron_sword",
            slot: 1,
            description: "One second of flight, and nothing can touch you during it.",
            cooldown: 9.0,
            activate: super_squid,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Ink Shotgun",
            item: "minecraft:iron_axe",
            slot: 2,
            description: "Seven ink sacs at once. All seven is 12 damage and almost never happens.",
            cooldown: 5.0,
            activate: ink_shotgun,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Fish Flurry",
            item: "minecraft:iron_shovel",
            slot: 3,
            description: "Fish erupt from the ground for four seconds. Hard to walk out of.",
            // `[VERIFIED]`: "a ridiculous 16 seconds cooldown to balance it".
            cooldown: 16.0,
            activate: fish_flurry,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Storm Squid",
            item: "minecraft:nether_star",
            slot: 8,
            description: "Fly, and call lightning down once a second.",
            cooldown: 1.0,
            activate: storm_squid,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// `[VERIFIED]` one second of flight; the invulnerability needs a damage-immune
/// window the game half does not model yet, so what lands here is the movement.
/// `[APPROXIMATED]` impulse.
fn super_squid(cast: &Cast<'_>) {
    cast.server.add_velocity(
        cast.player,
        cast.facing.0.normalize_or_zero() * 1.1 + Vec3::Y * 0.9,
    );
    cast.server.cue(cast.position.0, Cue::Charge);
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
}

/// `[VERIFIED]` 5x5x5 area, 2 damage per fish, four seconds. Modelled as one
/// splash rather than as individual dropped items, because the items would need
/// the host's entity system and the damage is the part that matters.
fn fish_flurry(cast: &Cast<'_>) {
    let at = cast.position.0 + cast.facing.0.normalize_or_zero() * 4.0;
    splash_at(cast, at, 2.5, 2.0, 0.7);
    cast.server.cue(at, Cue::Explosion);
}

/// `[APPROXIMATED]`: a lightning bolt a second for the crystal's duration needs
/// a repeating effect the ability layer does not have. One strike stands in.
fn storm_squid(cast: &Cast<'_>) {
    use crate::module::{ability::splash_at as splash_there, player::Position};

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
        splash_there(cast, at, 2.0, 6.0, 1.2);
        cast.server.cue(at, Cue::Explosion);
    }
}
