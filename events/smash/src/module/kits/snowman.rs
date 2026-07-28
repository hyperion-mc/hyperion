//! Snowman: an aura that buffs your melee and slows whoever stands in it.
//!
//! Arctic Aura is the kit: +1 damage against anyone standing on your snow, and
//! very little knockback so they stay there. Both are the wiki's.
//!
//! Stats verified: 6.0 damage rising to 7.0 in the aura, 12 armour points
//! (48%), 140% knockback taken, 0.3 regen, 5000 gems.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::module::{
    ability::{self, Cast, Observable, splash_at},
    effect::{self, Affliction},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    projectile::{Flight, Payload, fire},
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
        .sounds(KitSounds {
            select: "minecraft:entity.snow_golem.ambient",
            hurt: "minecraft:entity.snow_golem.hurt",
            death: "minecraft:entity.snow_golem.death",
        })
        .cost(5000)
        .skin(crate::kit_skin!("snowman"))
        .blurb("Own the ground you are standing on.")
        .mob("minecraft:snow_golem")
        .ability(AbilitySpec {
            name: "Blizzard",
            sound: "minecraft:entity.snow_golem.shoot",
            item: "minecraft:iron_sword",
            description: "Snowballs, endlessly. Low damage, good knockback, best at an edge.",
            cooldown: 0.4,
            energy_cost: Some(8.0),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: blizzard,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Ice Path",
            sound: "minecraft:block.snow.place",
            item: "minecraft:iron_axe",
            description: "A path of ice wherever you point, and a hop so you do not fall through.",
            cooldown: 8.0,
            proves: &[Observable::LaunchesCaster],
            activate: ice_path,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Arctic Aura",
            sound: "minecraft:entity.snow_golem.ambient",
            item: "minecraft:snow_block",
            description: "Snow around you. Slows them, and you hit a point harder on it.",
            cooldown: 2.0,
            energy_cost: Some(20.0),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: arctic_aura,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Snow Turret",
            sound: "minecraft:entity.snowball.throw",
            item: "minecraft:nether_star",
            description: "Snowmen that shoot for you. Twenty seconds, three of them.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
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
    cast.server.add_velocity(
        cast.player,
        Vec3::Y * 0.42 + cast.facing.0.normalize_or_zero() * 0.6,
    );
}

/// `[APPROXIMATED]`: the aura is modelled as a pulse rather than as placed snow,
/// because "who is standing on my snow" needs the host's blocks.
fn arctic_aura(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, 5.0, AURA_BONUS_DAMAGE, 0.2);
}

/// Twenty seconds of a six-snowball ring, the first one down the barrel.
///
/// The ring is turned to face the caster: it used to be laid out in absolute
/// world directions, so the ultimate hit whatever happened to be east of the
/// snowman and a target dead ahead was in the gap between two of the six unless
/// the fight happened to be lined up with +X. Nobody would have noticed from the
/// inside, because the ability does fire and does hit *something*.
///
/// `[WIKI]` three turrets for twenty seconds; `[SOURCE]` says one, and
/// `docs/smash-design.md` records the disagreement.
///
/// A turret is a thing that keeps shooting, which is the half that was missing:
/// the ability fired one ring of six snowballs and stopped. Without a spawned
/// mob to stand there and fire -- which needs the host's entity system -- the
/// ring on a beat for twenty seconds is what a player standing next to a
/// Snowman actually experiences, and it is the same ring.
fn snow_turret(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, TURRET_INTERVAL, turret_ring),
    );
}

/// `[APPROXIMATED]`. Blizzard's own cooldown is 0.4 s and a turret is meant to
/// be a second Snowman, so a ring a second is roughly three of them firing.
const TURRET_INTERVAL: f32 = 1.0;

fn turret_ring(cast: &Cast<'_>) {
    let flat = Vec3::new(cast.facing.0.x, 0.0, cast.facing.0.z).normalize_or_zero();
    // Looking straight down leaves no bearing to build a ring on.
    let forward = if flat == Vec3::ZERO { Vec3::Z } else { flat };

    for step in 0..6 {
        let angle = std::f32::consts::TAU * (step as f32) / 6.0;
        let (sin, cos) = angle.sin_cos();
        let direction = Vec3::new(
            forward.x.mul_add(cos, -(forward.z * sin)),
            0.0,
            forward.x.mul_add(sin, forward.z * cos),
        );
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
