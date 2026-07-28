//! Guardian: a grappler that marks one player and takes them apart.
//!
//! Target Laser is the kit and it is unusually well documented: melee rises
//! from 5 to 7 against the marked player, the mark needs them within ten blocks
//! and you on the ground, it lasts at most eight seconds, ends with a further
//! three damage, and puts the ability on a fifteen-second cooldown. All of that
//! is the wiki's.
//!
//! Stats verified: 5.0 damage rising to 8.0, 9 armour points (36%), 125%
//! knockback taken, 0.25 regen, 8000 gems.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        ability::{Cast, Observable, splash_at},
        damage::MatchClock,
        kit::{self, AbilitySpec, KitSounds, KitStats},
        player::{Player, Position},
        projectile::{Flight, Impact, Payload, fire},
    },
    server::{Cue, PlayerId, ServerHandle},
};

/// `[VERIFIED]`: "increased damage (5 -> 7)", "8 seconds at maximum",
/// "ending the ability with another 3 damage", "cooldown of 15 seconds",
/// "If no one is near you (within 10 blocks) ... you won't be able to use it".
pub const LASER_BONUS_DAMAGE: f32 = 2.0;
pub const LASER_SECONDS: f32 = 8.0;
pub const LASER_FINISH_DAMAGE: f32 = 3.0;
pub const LASER_RANGE: f32 = 10.0;

/// The mark, on the Guardian. Points at the victim and says when it lapses.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Marked {
    pub victim: Entity,
    pub until: f32,
}

#[derive(Component)]
pub struct Guardian;

impl Module for Guardian {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Guardian");
        world.component::<Marked>();

        kit::define(world, "Guardian", KitStats {
            melee_damage: 5.0,
            armor: 9.0,
            knockback_taken: 1.25,
            regen: 0.25,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            hurt: "minecraft:entity.guardian.hurt",
            death: "minecraft:entity.guardian.death",
        })
        .cost(8000)
        .skin(crate::kit_skin!("guardian"))
        .blurb("Pick somebody. They are now your problem and you are theirs.")
        .mob("minecraft:guardian")
        .ability(AbilitySpec {
            name: "Whirlpool Axe",
            sound: "minecraft:entity.player.splash.high_speed",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "A shard that pulls, like a weaker hook on a shorter cooldown.",
            // `[VERIFIED]` "its low recharge time of 5 seconds".
            cooldown: 5.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: whirlpool_axe,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Water Splash",
            sound: "minecraft:entity.generic.splash",
            item: "minecraft:iron_sword",
            slot: 2,
            description: "Bounce up, dragging everyone within five blocks with you.",
            // `[VERIFIED]` "Due to its cooldown of 12 seconds".
            cooldown: 12.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: water_splash,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Target Laser",
            sound: "minecraft:entity.guardian.attack",
            item: "minecraft:iron_pickaxe",
            slot: 3,
            description: "Mark someone within ten blocks. Everything hurts them more for eight \
                          seconds.",
            cooldown: 15.0,
            requires_ground: true,
            proves: &[Observable::BuffsMelee],
            activate: target_laser,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Tidal Wave",
            sound: "minecraft:entity.elder_guardian.curse",
            item: "minecraft:nether_star",
            slot: 8,
            description: "Everything in the water goes where the water goes.",
            cooldown: 20.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: tidal_wave,
            ..AbilitySpec::DEFAULT
        })
        .register();

        // The mark lapses on its own, and lapsing costs the victim three more
        // damage whether or not the Guardian is still nearby.
        world
            .system_named::<(&Marked, &PlayerId)>("smash::kits::guardian::laser_expiry")
            .each_entity(|guardian, (marked, _)| {
                let world = guardian.world();
                let now = world.get::<&MatchClock>(|clock| clock.0);
                if now < marked.until {
                    return;
                }
                let victim = world.entity_from_id(marked.victim);
                guardian.remove(Marked::id());
                if !victim.is_alive() {
                    return;
                }
                crate::module::damage::hurt(victim, crate::module::damage::Damaged {
                    attacker: Some(guardian.id()),
                    amount: LASER_FINISH_DAMAGE,
                    knockback: crate::module::knockback::Knockback::from(Vec3::ZERO).times(0.0),
                    kind: crate::module::damage::DamageKind::Ability,
                });
            });
    }
}

fn whirlpool_axe(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            // "It moves rather slow and has less pulling force" than Iron Hook,
            // whose projectile travels at 20.
            velocity: cast.facing.0.normalize_or_zero() * 14.0,
            gravity: 0.0,
            seconds_left: 1.4,
            radius: 0.6,
        },
        Payload::new(3.0, 0.8).then(reel),
    );
}

fn reel(impact: &Impact<'_>) {
    let (Some(to), Some(from), Some(id)) = (
        impact.shooter.and_then(|s| s.try_get::<&Position>(|p| p.0)),
        impact.victim.try_get::<&Position>(|p| p.0),
        impact.victim.try_get::<&PlayerId>(|p| *p),
    ) else {
        return;
    };
    let pull = (to - from).normalize_or_zero() * 1.2 + Vec3::Y * 0.5;
    impact
        .world
        .get::<&ServerHandle>(|server| server.add_velocity(id, pull));
}

/// `[VERIFIED]` "pulls players within a 5 block radius to you as well, doing up
/// to 11 damage to them when landing".
fn water_splash(cast: &Cast<'_>) {
    cast.server.add_velocity(cast.player, Vec3::Y * 0.9);
    splash_at(cast, cast.position.0, 5.0, 11.0, 1.4);
    cast.server.cue(cast.position.0, Cue::Explosion);
}

fn target_laser(cast: &Cast<'_>) {
    let now = cast.world.get::<&MatchClock>(|clock| clock.0);
    let caster = cast.caster.id();
    let mut nearest: Option<(f32, Entity)> = None;
    cast.world
        .query::<&Position>()
        .with(Player::id())
        .build()
        .each_entity(|entity, position| {
            if entity.id() == caster {
                return;
            }
            let distance = position.0.distance(cast.position.0);
            if distance <= LASER_RANGE && nearest.is_none_or(|(best, _)| distance < best) {
                nearest = Some((distance, entity.id()));
            }
        });

    let Some((_, victim)) = nearest else {
        return;
    };
    cast.caster.set(Marked {
        victim,
        until: now + LASER_SECONDS,
    });
    // 5 -> 7 against the marked player, and against nobody else. This is what
    // `MeleeBonus::against` exists for.
    cast.caster.set(crate::module::damage::MeleeBonus {
        flat: LASER_BONUS_DAMAGE,
        against: Some(victim),
        until: now + LASER_SECONDS,
    });
}

fn tidal_wave(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, 10.0, 12.0, 2.2);
}
