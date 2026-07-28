//! Spider: the rushdown kit.
//!
//! One of two kits with two passives, and the reason `jump_control` exists:
//! Spider Leap sends the double jump where you are looking rather than straight
//! up, which is what turns Spider from a melee kit into a mobile one.
//!
//! Stats verified against the wiki's kit table: 6.0 damage, 11 armour points
//! (44% reduction), 150% knockback taken, 0.25 regen. Ability numbers are the
//! wiki's where it gives them -- Needler is six arrows -- and approximated
//! where it only describes ("above average damage"); each is marked below.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        ability::{Cast, Observable, splash},
        kit::{self, AbilitySpec, KitStats},
        projectile::{Flight, Payload, fire},
    },
    server::Cue,
};

#[derive(Component)]
pub struct Spider;

impl Module for Spider {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Spider");

        kit::define(world, "Spider", KitStats {
            melee_damage: 6.0,
            armor: 11.0,
            knockback_taken: 1.50,
            regen: 0.25,
            jump_power: 1.0,
            // Spider Leap. The wiki lists jump power as "1.0 (Direct)", which is
            // this flag.
            jump_control: true,
            ..KitStats::default()
        })
        .cost(0)
        .blurb("Fast, fragile and everywhere at once. Leap where you look.")
        .ability(AbilitySpec {
            name: "Needler",
            item: "minecraft:iron_sword",
            slot: 1,
            description: "Spray six needles. They poison, which armour does not stop.",
            cooldown: 6.0,
            charge_time: Some(1.0),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: needler,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Spin Web",
            item: "minecraft:iron_axe",
            slot: 2,
            description: "Launch forward, trailing web. Mostly a way back onto the map.",
            cooldown: 8.0,
            proves: &[Observable::LaunchesCaster],
            activate: spin_web,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Spiders Nest",
            item: "minecraft:nether_star",
            slot: 8,
            description: "A dome of web. Everything you hit heals you.",
            cooldown: 1.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: spiders_nest,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// Six arrows in a cone. `[VERIFIED]` count; `[APPROXIMATED]` damage, which the
/// wiki calls only "a good amount at close range".
fn needler(cast: &Cast<'_>) {
    const NEEDLES: usize = 6;
    const SPREAD: f32 = 0.12;

    let forward = cast.facing.0.normalize_or_zero();
    for index in 0..NEEDLES {
        // A fixed fan rather than a random one: a needle you can aim is the
        // difference between an edge-guard and a coin toss, and a test can
        // predict it.
        let offset = (index as f32 - (NEEDLES as f32 - 1.0) / 2.0) * SPREAD;
        let direction =
            (forward + Vec3::new(-forward.z, 0.0, forward.x) * offset).normalize_or_zero();
        fire(
            cast.world,
            cast.caster,
            Flight {
                position: cast.position.0,
                velocity: direction * 34.0,
                gravity: 12.0,
                seconds_left: 1.5,
                radius: 0.6,
            },
            Payload::new(1.5, 0.35),
        );
    }
}

/// A leap that leaves web behind. `[APPROXIMATED]`: the recovery distance is
/// tuned to "decent horizontal and vertical", not to a published number.
fn spin_web(cast: &Cast<'_>) {
    let launch = cast.facing.0.normalize_or_zero() * 1.4 + Vec3::Y * 0.45;
    cast.server.add_velocity(cast.player, launch);
    cast.server.cue(cast.position.0, Cue::Charge);
}

/// `[APPROXIMATED]`: the dome traps, which needs block writes the game half
/// cannot make. The damage and the one-second recharge are the wiki's.
fn spiders_nest(cast: &Cast<'_>) {
    splash(cast, 6.0, 4.0, 0.8);
}
