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
use hyperion::simulation::entity_kind::EntityKind;

use crate::module::{
    ability::{self, Cast, Observable, splash_at},
    damage::MeleeBonus,
    effect::{self, Affliction},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    projectile::{Flight, Payload, Visual, fire},
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
            description: "Become a mooshroom for twenty seconds: more damage, five more hearts, \
                          and a herd every two seconds.",
            cooldown: 20.0,
            proves: &[
                Observable::HealsCaster,
                Observable::BuffsMelee,
                Observable::HurtsTarget,
                // The herd on each beat is the kit's own Angry Herd, and a cow
                // that hits you moves you.
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
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
            Visual(EntityKind::Cow),
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

/// `[VERIFIED]` "+1 Damage".
pub const MOOSHROOM_BONUS_DAMAGE: f32 = 1.0;

/// `[APPROXIMATED]`. The wiki's "faster abilities" needs a cooldown scale the
/// ability layer does not carry, so what stands in for it is the kit's own herd
/// arriving on a beat -- which is what a Cow whose abilities are coming faster
/// than usual looks like from the outside.
const MOOSHROOM_INTERVAL: f32 = 2.0;

/// The kit's own maximum health, which the bonus is measured against.
///
/// Read from the kit prefab and not from the player, because the player's
/// maximum is the thing this ability changes and reading it back would compound:
/// two crystals in one match used to leave a Cow on forty hearts and a third on
/// fifty.
fn base_health(cast: &Cast<'_>) -> f32 {
    use crate::{flecs_ext::EntityViewExt, module::kit::Playing};
    cast.caster
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| stats.max_health))
        .unwrap_or(20.0)
}

/// `[VERIFIED]` "+1 Damage ... 5 more hearts", for twenty seconds.
///
/// The transformation itself needs the host's entity type machinery. What lands
/// is everything else -- and, the part that was missing, it is taken back when
/// the twenty seconds are up. The maximum used to be raised permanently, so the
/// ultimate was strictly stronger than the wiki describes and got stronger again
/// with every crystal.
fn mooshroom_madness(cast: &Cast<'_>) {
    use crate::module::player::Health;

    let (current, max) = cast.caster.get::<&mut Health>(|health| {
        health.max = base_health(cast) + MOOSHROOM_BONUS_HEALTH;
        health.current = health.max;
        (health.current, health.max)
    });
    cast.server.set_health(cast.player, current, max);

    // `against: None`, because the wiki's +1 is against everybody -- unlike
    // Guardian's mark, which is the other user of this component. The deadline
    // is the ability layer's own, so the bonus and the mode cannot disagree
    // about when they end.
    let now = cast.world.cloned::<&crate::module::damage::MatchClock>().0;
    cast.caster.set(MeleeBonus {
        flat: MOOSHROOM_BONUS_DAMAGE,
        against: None,
        until: now + ability::ULTIMATE_SECONDS,
    });

    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, MOOSHROOM_INTERVAL, angry_herd)
            .undone_by(shrink_back),
    );
}

/// Put the Cow back the size it was.
///
/// Runs however the mode ends -- expiry, a respawn clearing effects, the holder
/// dying -- because [`crate::module::effect::Ends`] has exactly one teardown
/// path. Current health is clamped rather than left above a maximum that just
/// dropped, which `tests/properties.rs` checks on every tick.
fn shrink_back(cast: &Cast<'_>) {
    use crate::module::player::Health;

    let base = base_health(cast);
    let (current, max) = cast.caster.get::<&mut Health>(|health| {
        health.max = base;
        health.current = health.current.min(base);
        (health.current, health.max)
    });
    cast.server.set_health(cast.player, current, max);
}
