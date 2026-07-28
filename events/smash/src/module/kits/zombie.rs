//! Zombie: the ranged-and-melee generalist.
//!
//! Stats verified: 6.0 damage, 10 armour points (40%), 125% knockback taken,
//! 0.25 regen, 6000 gems. Every ability number is `[APPROXIMATED]`: the wiki
//! names the four abilities and describes them, and gives no figures at all.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::entity_kind::EntityKind;

use crate::{
    module::{
        ability::{self, Cast, Observable, splash_at},
        effect::{self, Affliction},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        player::Position,
        projectile::{Flight, Impact, Payload, Visual, fire},
        visuals,
    },
    server::{PlayerId, ServerHandle},
};

#[derive(Component)]
pub struct Zombie;

impl Module for Zombie {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Zombie");

        kit::define(world, "Zombie", KitStats {
            melee_damage: 6.0,
            armor: 10.0,
            knockback_taken: 1.25,
            regen: 0.25,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.zombie.ambient",
            hurt: "minecraft:entity.zombie.hurt",
            death: "minecraft:entity.zombie.death",
        })
        .cost(6000)
        .skin(crate::kit_skin!("zombie"))
        .blurb("Something for every range, and nothing outstanding at any of them.")
        .mob("minecraft:zombie")
        .ability(AbilitySpec {
            name: "Bile Blaster",
            sound: "minecraft:entity.witch.throw",
            item: "minecraft:iron_axe",
            description: "Spray bile. It lands where you point and hurts what it lands on.",
            cooldown: 7.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: bile_blaster,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Deaths Grasp",
            sound: "minecraft:entity.fishing_bobber.retrieve",
            item: "minecraft:bow",
            description: "An arrow that drags whoever it hits back to you.",
            cooldown: 9.0,
            charge_time: Some(1.0),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: deaths_grasp,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Night of the Living Dead",
            sound: "minecraft:entity.zombie.ambient",
            item: "minecraft:nether_star",
            description: "The dead get up around you, and keep getting up for twenty seconds.",
            cooldown: 20.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
            activate: night_of_the_living_dead,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

fn bile_blaster(cast: &Cast<'_>) {
    const BLOBS: usize = 4;
    let forward = cast.facing.0.normalize_or_zero();
    let side = Vec3::new(-forward.z, 0.0, forward.x);
    for index in 0..BLOBS {
        let offset = (index as f32 - (BLOBS as f32 - 1.0) / 2.0) * 0.14;
        fire(
            cast.world,
            cast.caster,
            // `[APPROXIMATED]`: bile has no entity; a snowball is the closest
            // always-rendered blob.
            Visual(EntityKind::Snowball),
            Flight {
                position: cast.position.0,
                velocity: (forward + side * offset).normalize_or_zero() * 20.0,
                gravity: 14.0,
                seconds_left: 1.6,
                radius: 0.7,
            },
            Payload::new(2.5, 0.5),
        );
    }
}

fn deaths_grasp(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Visual(EntityKind::Arrow),
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 16.0f32.mul_add(cast.charge, 24.0),
            gravity: 6.0,
            seconds_left: 2.0,
            radius: 0.6,
        },
        Payload::new(4.0, 0.4).then(drag_back),
    );
}

fn drag_back(impact: &Impact<'_>) {
    let (Some(to), Some(from), Some(id)) = (
        impact.shooter.and_then(|s| s.try_get::<&Position>(|p| p.0)),
        impact.victim.try_get::<&Position>(|p| p.0),
        impact.victim.try_get::<&PlayerId>(|p| *p),
    ) else {
        return;
    };
    let pull = (to - from).normalize_or_zero() * 1.6 + Vec3::Y * 0.5;
    impact
        .world
        .get::<&ServerHandle>(|server| server.add_velocity(id, pull));
}

/// `[APPROXIMATED]` throughout; the wiki names the ability and describes it and
/// gives no figures.
///
/// "The dead get up around you" is a horde, and a horde is not one moment. A
/// wave every second and a half for twenty seconds is the closest the kit gets
/// to one without a spawned mob to walk around, and it makes standing next to a
/// Zombie holding a crystal the mistake the name implies.
fn night_of_the_living_dead(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, HORDE_INTERVAL, horde_wave),
    );
}

const HORDE_INTERVAL: f32 = 1.5;

/// Per wave, and there are thirteen of them, so it is a fraction of what the
/// single burst dealt.
const HORDE_DAMAGE: f32 = 2.5;

fn horde_wave(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, 7.0, HORDE_DAMAGE, 1.6);
    cast.server.particles(visuals::blast(cast.position.0));
}
