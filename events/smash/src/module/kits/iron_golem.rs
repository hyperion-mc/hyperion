//! Iron Golem: the melee kit.
//!
//! Highest melee damage and armour in the game, permanently slowed, and with no
//! ranged option at all — every ability exists to close distance or to punish
//! someone who let it. Iron Hook is the kit: land it and the victim arrives in
//! melee range whether they wanted to or not.
//!
//! Numbers from the `SMASH_KITS` spreadsheet dump unless noted.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        ability::{Cast, Observable, splash, splash_at},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        player::Position,
        projectile::{Flight, Impact, Payload, fire},
    },
    server::Cue,
};

/// `PerkSlow(0)`, reapplied every four seconds. The armour is meant to be paid
/// for.
pub const PERMANENT_SLOWNESS: u8 = 1;

#[derive(Component)]
pub struct IronGolem;

impl Module for IronGolem {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::IronGolem");

        kit::define(world, "Iron Golem", KitStats {
            melee_damage: 7.0,
            armor: 16.0,
            knockback_taken: 1.00,
            regen: 0.20,
            hunger_interval: 7.75,
            jump_power: 0.9,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            hurt: "minecraft:entity.iron_golem.hurt",
            death: "minecraft:entity.iron_golem.death",
        })
        .blurb("Command space in the arena. Pull enemies in, then hit like a truck.")
        .ability(AbilitySpec {
            name: "Fissure",
            sound: "minecraft:block.deepslate.break",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "Split the ground in a line, launching whoever it reaches.",
            cooldown: 8.0,
            requires_ground: true,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: fissure,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Iron Hook",
            sound: "minecraft:block.chain.place",
            item: "minecraft:iron_pickaxe",
            slot: 2,
            description: "Throw a hook. On a hit it drags them to you.",
            cooldown: 8.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: iron_hook,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Seismic Slam",
            sound: "minecraft:entity.iron_golem.attack",
            item: "minecraft:iron_shovel",
            slot: 3,
            description: "Leap, then land hard. Everything nearby goes flying.",
            cooldown: 7.0,
            requires_ground: true,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: seismic_slam,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Earthquake",
            sound: "minecraft:entity.ravager.roar",
            item: "minecraft:nether_star",
            slot: 8,
            description: "Shake the whole map. Anyone touching the ground pays.",
            cooldown: 16.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: earthquake,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// Damage rises along the fissure: `4 + column`, over fourteen blocks.
///
/// Mineplex ran a real block-by-block wall with a four-tick delay before the
/// launch. Without the host's world to place blocks in, the columns are
/// resolved immediately as a swept series of splashes; the damage ramp and the
/// range are the original's.
fn fissure(cast: &Cast<'_>) {
    const LENGTH: usize = 14;
    const HIT_RADIUS: f32 = 1.5;

    let step = Vec3::new(cast.facing.0.x, 0.0, cast.facing.0.z).normalize_or_zero();
    for column in 0..LENGTH {
        let at = cast.position.0 + step * column as f32;
        splash_at(cast, at, HIT_RADIUS, 4.0 + column as f32, 1.0);
    }
    cast.server.cue(cast.position.0, Cue::Explosion);
}

/// A slow, heavy projectile that reels its victim in.
fn iron_hook(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 20.0,
            gravity: 0.0,
            seconds_left: 1.2,
            radius: 0.5,
        },
        Payload::new(4.0, 1.0).then(reel_in),
    );
}

fn reel_in(impact: &Impact<'_>) {
    let Some(shooter) = impact.shooter else {
        return;
    };
    let Some(to) = shooter.try_get::<&Position>(|p| p.0) else {
        return;
    };
    let Some(from) = impact.victim.try_get::<&Position>(|p| p.0) else {
        return;
    };
    let Some(id) = impact.victim.try_get::<&crate::server::PlayerId>(|p| *p) else {
        return;
    };

    // Mineplex's `velocity(2, yBase 0.8, yMax 1.5)`: hard pull with enough lift
    // to clear whatever the victim was standing behind.
    let pull = (to - from).normalize_or_zero() * 2.0 + Vec3::Y * 0.8;
    impact
        .world
        .get::<&crate::server::ServerHandle>(|server| server.add_velocity(id, pull));
}

/// The kit's panic button and its combo finisher.
fn seismic_slam(cast: &Cast<'_>) {
    splash(cast, 8.0, 10.5, 2.4);
}

/// Hits every grounded player on the map, wherever they are.
///
/// Each victim is hurt once, by id. Splashing at each of their positions in
/// turn double-hit anyone standing near somebody else, which made the ultimate
/// swing between three damage and twelve depending on how bunched up the arena
/// happened to be.
fn earthquake(cast: &Cast<'_>) {
    use crate::module::{
        damage::{DamageKind, Damaged, hurt},
        knockback::Knockback,
        player::{Health, OnGround, Player},
    };

    const DAMAGE: f32 = 3.0;

    let caster = cast.caster.id();
    let mut victims = Vec::new();
    cast.world
        .query::<(&OnGround, &Health)>()
        .with(Player::id())
        .build()
        .each_entity(|entity, (ground, health)| {
            if entity.id() != caster && ground.0 && !health.is_dead() {
                victims.push(entity.id());
            }
        });

    for victim in victims {
        hurt(cast.world.entity_from_id(victim), Damaged {
            attacker: Some(caster),
            amount: DAMAGE,
            // Away from the golem. A shockwave centred on each victim in turn
            // is what the previous version computed, and knockback away from
            // the point you are standing on normalises to nothing.
            knockback: Knockback::from(cast.position.0),
            kind: DamageKind::Ability,
        });
    }
    cast.server.cue(cast.position.0, Cue::Explosion);
}
