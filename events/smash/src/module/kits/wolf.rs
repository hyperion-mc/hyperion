//! Wolf: melee that gets stronger the longer it stays on you.
//!
//! Ravage is the kit. Base damage is 5, the lowest of the melee kits, and every
//! landed hit stacks a bonus toward a ceiling of 8 that decays three seconds
//! after the last one. All three numbers are the wiki's.
//!
//! Stats verified: 5.0 damage rising to 8.0, 9 armour points (36%), and the
//! knockback figure -- the kit table says 150%, the kit's own page says 160%.
//! The kit page is the more specific source and is the one used here; the
//! disagreement is recorded in docs/smash-design.md.

use flecs_ecs::prelude::*;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{Cast, Observable},
        damage::{DamageKind, Damaged, MeleeBonus},
        kit::{self, AbilitySpec, KitName, KitSounds, KitStats, Playing},
        player::Player,
        projectile::{Flight, Impact, Payload, fire},
    },
};

/// Damage added per landed melee hit, and the ceiling it climbs to.
pub const RAVAGE_PER_STACK: f32 = 1.0;
pub const RAVAGE_MAX_DAMAGE: f32 = 8.0;
/// How long a stack lasts. The wiki: "Each attack bonus lasts for 3 seconds."
pub const RAVAGE_DECAY_SECS: f32 = 3.0;

/// Slowness left on a player a cub has tackled. Wolf Strike checks it, because
/// the combo is the kit's whole payoff.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Tackled(pub f32);

/// Seconds of near-immobility a cub leaves behind.
pub const TACKLE_SLOW_SECS: f32 = 5.0;

#[derive(Component)]
pub struct Wolf;

impl Module for Wolf {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Wolf");
        world.component::<Tackled>();

        kit::define(world, "Wolf", KitStats {
            melee_damage: 5.0,
            armor: 9.0,
            knockback_taken: 1.60,
            regen: 0.25,
            jump_power: 1.0,
            jump_control: true,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            hurt: "minecraft:entity.wolf.hurt",
            death: "minecraft:entity.wolf.death",
        })
        .cost(4000)
        .blurb("Stick to somebody and every hit lands harder than the last.")
        .ability(AbilitySpec {
            name: "Cub Tackle",
            sound: "minecraft:entity.baby_wolf.ambient",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "Throw a cub. Whoever it lands on can barely move for five seconds.",
            cooldown: 8.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: cub_tackle,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Wolf Strike",
            sound: "minecraft:entity.wolf.growl",
            item: "minecraft:iron_shovel",
            slot: 2,
            description: "Launch at what you are looking at. Triple knockback on a tackled target.",
            cooldown: 7.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: wolf_strike,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Frenzy",
            sound: "minecraft:entity.wolf_angry.growl",
            item: "minecraft:nether_star",
            slot: 8,
            description: "Twenty seconds of everything at once.",
            cooldown: 20.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: frenzy,
            ..AbilitySpec::DEFAULT
        })
        .register();

        // Ravage. A melee hit by a Wolf adds a stack; the stack decays rather
        // than being cleared, so a Wolf who keeps pressure on keeps the bonus.
        world
            .observer_named::<Damaged, ()>("smash::kits::wolf::ravage")
            .with(Player::id())
            .each_iter(|it, _index, ()| {
                let event = *it.param();
                if event.kind != DamageKind::Melee {
                    return;
                }
                let world = it.world();
                let Some(attacker) = event.attacker else {
                    return;
                };
                let attacker = world.entity_from_id(attacker);
                if !plays(attacker, "Wolf") {
                    return;
                }
                let now = world.get::<&crate::module::damage::MatchClock>(|clock| clock.0);
                let ceiling = RAVAGE_MAX_DAMAGE - 5.0;
                let current = attacker
                    .try_get::<&MeleeBonus>(|bonus| bonus.applies_to(attacker.id(), now))
                    // `applies_to` is asked about the attacker itself only to
                    // reuse the expiry check; Ravage is never targeted, so any
                    // entity would answer the same.
                    .unwrap_or(0.0);
                attacker.set(MeleeBonus {
                    flat: (current + RAVAGE_PER_STACK).min(ceiling),
                    against: None,
                    until: now + RAVAGE_DECAY_SECS,
                });
            });
    }
}

fn plays(player: EntityView<'_>, kit: &str) -> bool {
    player
        .find_target(Playing, |prefab| {
            prefab.try_get::<&KitName>(|name| name.0 == kit) == Some(true)
        })
        .is_some()
}

fn cub_tackle(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 18.0,
            gravity: 8.0,
            seconds_left: 2.0,
            radius: 0.8,
        },
        Payload::new(3.0, 0.4).then(tackle),
    );
}

fn tackle(impact: &Impact<'_>) {
    let now = impact
        .world
        .get::<&crate::module::damage::MatchClock>(|clock| clock.0);
    impact.victim.set(Tackled(now + TACKLE_SLOW_SECS));
}

/// The wiki: base knockback dealt 200%, rising to 300% against a target still
/// slowed by Cub Tackle. Both `[VERIFIED]`; the launch impulse is
/// `[APPROXIMATED]`.
fn wolf_strike(cast: &Cast<'_>) {
    use crate::module::{ability::splash_at, damage::MatchClock, player::Position};

    const BASE_KNOCKBACK: f32 = 2.0;
    const TACKLED_KNOCKBACK: f32 = 3.0;
    const REACH: f32 = 4.0;

    let now = cast.world.get::<&MatchClock>(|clock| clock.0);
    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * REACH;

    cast.server
        .add_velocity(cast.player, cast.facing.0.normalize_or_zero() * 1.6);

    let caster = cast.caster.id();
    let mut combo = false;
    cast.world
        .query::<&Position>()
        .with(Player::id())
        .build()
        .each_entity(|entity, position| {
            if entity.id() == caster || position.0.distance(ahead) > REACH {
                return;
            }
            if entity.try_get::<&Tackled>(|t| now < t.0) == Some(true) {
                combo = true;
            }
        });

    let knockback = if combo {
        TACKLED_KNOCKBACK
    } else {
        BASE_KNOCKBACK
    };
    splash_at(cast, ahead, REACH, 6.0, knockback);
}

/// "Speed III, Regeneration III, and Strength III, and all your abilities
/// recharge much faster. Lasts 20 seconds." The status effects need the host's
/// potion machinery, so what is modelled here is the damage burst.
/// `[APPROXIMATED]`.
fn frenzy(cast: &Cast<'_>) {
    use crate::module::ability::splash;
    splash(cast, 5.0, 8.0, 1.5);
}
