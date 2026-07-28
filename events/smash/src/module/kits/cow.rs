//! Cow: the heavyweight.
//!
//! Second-highest armour, the lowest knockback taken of any kit at 110%, and a
//! passive that makes sprinting into somebody a real plan. Every cooldown here
//! is the wiki's, which for once gives them explicitly.
//!
//! Stats verified: 6.0 damage (the kit table says 6.5, the kit page says 6.0 --
//! the page is used, and the disagreement is recorded in the design doc),
//! 13 armour points, 110% knockback taken, 0.25 regen, 6000 gems.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::module::{
    ability::{Cast, Observable, splash_at},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    projectile::{Flight, Payload, fire},
};

/// `[VERIFIED]`: "you will slowly gain speed levels (up to 4)".
pub const STAMPEDE_MAX_LEVEL: u8 = 4;

#[derive(Component)]
pub struct Cow;

impl Module for Cow {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Cow");

        kit::define(world, "Cow", KitStats {
            melee_damage: 6.0,
            armor: 13.0,
            knockback_taken: 1.10,
            regen: 0.25,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.cow.ambient",
            hurt: "minecraft:entity.cow.hurt",
            death: "minecraft:entity.cow.death",
        })
        .cost(6000)
        .skin(crate::kit_skin!("cow"))
        .blurb("Hard to move and hard to stop once it is moving.")
        .mob("minecraft:cow")
        .ability(AbilitySpec {
            name: "Angry Herd",
            sound: "minecraft:entity.cow.ambient",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "Five cows in a line. Each one can hit you.",
            // `[VERIFIED]` "(Cooldown: 13 seconds)".
            cooldown: 13.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: angry_herd,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Milk Spiral",
            sound: "minecraft:entity.cow.milk",
            item: "minecraft:iron_shovel",
            slot: 2,
            description: "A helix of milk that carries you with it. Hits at most two people.",
            // `[VERIFIED]` "(Cooldown: 11 seconds)".
            cooldown: 11.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: milk_spiral,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Mooshroom Madness",
            sound: "minecraft:entity.mooshroom.convert",
            item: "minecraft:nether_star",
            slot: 8,
            description: "Become a mooshroom: more damage, five more hearts, faster abilities.",
            cooldown: 20.0,
            proves: &[Observable::HealsCaster],
            activate: mooshroom_madness,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// `[VERIFIED]` five cows; `[APPROXIMATED]` damage.
fn angry_herd(cast: &Cast<'_>) {
    const COWS: usize = 5;
    let forward = cast.facing.0.normalize_or_zero();
    let side = Vec3::new(-forward.z, 0.0, forward.x);
    for index in 0..COWS {
        let offset = (index as f32 - (COWS as f32 - 1.0) / 2.0) * 0.6;
        fire(
            cast.world,
            cast.caster,
            Flight {
                position: cast.position.0 + side * offset,
                velocity: forward * 14.0,
                gravity: 0.0,
                seconds_left: 2.5,
                radius: 0.9,
            },
            Payload::new(4.0, 1.1),
        );
    }
}

/// `[APPROXIMATED]` damage; the "carries you along" part is the wiki's.
fn milk_spiral(cast: &Cast<'_>) {
    let forward = cast.facing.0.normalize_or_zero();
    cast.server.add_velocity(cast.player, forward * 1.3);
    for step in 1..=6 {
        splash_at(
            cast,
            cast.position.0 + forward * (step as f32 * 1.5),
            1.8,
            3.0,
            0.9,
        );
    }
}

/// `[VERIFIED]` "5 more hearts".
pub const MOOSHROOM_BONUS_HEALTH: f32 = 10.0;

/// `[VERIFIED]` "+1 Damage ... 5 more hearts"; the transformation itself needs
/// the host's entity type machinery, so what lands is the heal.
fn mooshroom_madness(cast: &Cast<'_>) {
    use crate::{
        flecs_ext::EntityViewExt,
        module::{
            kit::{KitStats, Playing},
            player::Health,
        },
    };

    // Set against the kit's own maximum rather than added to whatever the
    // player currently has. Adding compounds: the crystal can be picked up more
    // than once in a match, and two Mooshroom Madnesses used to leave a Cow on
    // forty hearts and a third on fifty.
    let base = cast
        .caster
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| stats.max_health))
        .unwrap_or(20.0);

    let (current, max) = cast.caster.get::<&mut Health>(|health| {
        health.max = base + MOOSHROOM_BONUS_HEALTH;
        health.current = health.max;
        (health.current, health.max)
    });
    cast.server.set_health(cast.player, current, max);
}
