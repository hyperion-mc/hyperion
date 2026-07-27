//! Snowman: an aura that buffs your melee and slows whoever stands in it.
//!
//! Arctic Aura is the kit: +1 damage against anyone standing on your snow, and
//! very little knockback so they stay there. Both are the wiki's.
//!
//! Stats verified: 6.0 damage rising to 7.0 in the aura, 12 armour points
//! (48%), 140% knockback taken, 0.3 regen, 5000 gems.

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

/// `[VERIFIED]`: "You also deal 1 more damage to mobs who are on your snow."
pub const AURA_BONUS_DAMAGE: f32 = 1.0;

#[derive(Component)]
pub struct Snowman;

impl Module for Snowman {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Snowman");

        kit::define(world, "Snowman", KitStats {
            melee_damage: 6.0,
            armor: 12.0,
            knockback_taken: 1.40,
            regen: 0.30,
            // Blizzard and Arctic Aura both "draw from your Experience Bar".
            energy: Some((100.0, 18.0)),
            ..KitStats::default()
        })
        .cost(5000)
        .blurb("Own the ground you are standing on.")
        .ability(AbilitySpec {
            name: "Blizzard",
            item: "minecraft:iron_sword",
            slot: 1,
            description: "Snowballs, endlessly. Low damage, good knockback, best at an edge.",
            cooldown: 0.4,
            energy_cost: Some(8.0),
            activate: blizzard,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Ice Path",
            item: "minecraft:iron_axe",
            slot: 2,
            description: "A path of ice wherever you point, and a hop so you do not fall through.",
            cooldown: 8.0,
            activate: ice_path,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Arctic Aura",
            item: "minecraft:snow_block",
            slot: 3,
            description: "Snow around you. Slows them, and you hit a point harder on it.",
            cooldown: 2.0,
            energy_cost: Some(20.0),
            activate: arctic_aura,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Snow Turret",
            item: "minecraft:nether_star",
            slot: 8,
            description: "A snowman that shoots for you. Twenty seconds, three of them.",
            cooldown: 1.0,
            activate: snow_turret,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

fn blizzard(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 22.0,
            gravity: 12.0,
            seconds_left: 1.4,
            radius: 0.6,
        },
        Payload::new(1.5, 0.9),
    );
}

/// `[VERIFIED]`: "you will bounce 1 block into the air to avoid falling through
/// the path when it is made". The ice blocks themselves need host block writes,
/// which is the one thing the seam does not carry; the hop is what lands.
fn ice_path(cast: &Cast<'_>) {
    cast.server
        .add_velocity(cast.player, Vec3::Y * 0.42 + cast.facing.0.normalize_or_zero() * 0.6);
    cast.server.cue(cast.position.0, Cue::Charge);
}

/// `[APPROXIMATED]`: the aura is modelled as a pulse rather than as placed snow,
/// because "who is standing on my snow" needs the host's blocks.
fn arctic_aura(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, 5.0, AURA_BONUS_DAMAGE, 0.2);
}

fn snow_turret(cast: &Cast<'_>) {
    for step in 0..6 {
        let angle = std::f32::consts::TAU * (step as f32) / 6.0;
        let direction = Vec3::new(angle.cos(), 0.0, angle.sin());
        fire(
            cast.world,
            cast.caster,
            Flight {
                position: cast.position.0,
                velocity: direction * 20.0,
                gravity: 8.0,
                seconds_left: 1.6,
                radius: 0.6,
            },
            Payload::new(2.0, 0.8),
        );
    }
}
