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
use hyperion::{effects::Status, simulation::entity_kind::EntityKind};
use hyperion_minecraft_proto::generated::registry::MobEffect;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{self, Cast, Observable},
        damage::{DamageKind, Damaged, MeleeBonus},
        effect::{self, Affliction},
        kit::{self, AbilitySpec, KitName, KitSounds, KitStats, Playing},
        player::Player,
        projectile::{Flight, Impact, Payload, Visual, fire},
    },
    server::{PlayerId, ServerHandle},
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

/// The Slowness amplifier Cub Tackle applies. Zero-based on the wire, the way
/// the game counts it, so `5` is Slowness VI -- the level at which a player can
/// no longer move, which is what the tooltip's "can barely move" means. See
/// [`hyperion::effects::Status`].
pub const TACKLE_SLOW_AMPLIFIER: u8 = 5;

/// The immobilising slow a cub leaves on whoever it lands on.
///
/// Exposed so a wire test can assert the exact effect the ability sends rather
/// than a re-derivation of it, the way `hyperion`'s own `play_mob_effect`
/// differential pins the bytes against Mojang's encoder.
pub fn tackle_slow() -> Status {
    Status::new(MobEffect::Slowness, TACKLE_SLOW_AMPLIFIER).seconds(TACKLE_SLOW_SECS)
}

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
            select: "minecraft:entity.wolf.ambient",
            hurt: "minecraft:entity.wolf.hurt",
            death: "minecraft:entity.wolf.death",
        })
        .cost(4000)
        .skin(crate::kit_skin!("wolf"))
        .blurb("Stick to somebody and every hit lands harder than the last.")
        .mob("minecraft:wolf")
        .ability(AbilitySpec {
            name: "Cub Tackle",
            sound: "minecraft:entity.baby_wolf.ambient",
            item: "minecraft:iron_axe",
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
            description: "Twenty seconds of everything at once: harder hits, and a lunge every \
                          two seconds.",
            cooldown: 20.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
                Observable::BuffsMelee,
                Observable::Sustains,
            ],
            activate: frenzy,
            ..AbilitySpec::DEFAULT
        })
        .register();

        // Ravage. A melee hit by a Wolf adds a stack; the stack decays rather
        // than being cleared, so a Wolf who keeps pressure on keeps the bonus.
        world
            .observer_named::<Damaged, ()>("ravage")
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
        Visual(EntityKind::Wolf),
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
    // The marker Wolf Strike reads to know its combo target is still slowed.
    impact.victim.set(Tackled(now + TACKLE_SLOW_SECS));

    // And the real slow, alongside the marker rather than instead of it: a
    // `Slowness VI` the client applies to its own movement prediction, so the
    // victim actually can barely move rather than only appearing to a bystander.
    let Some(player) = impact.victim.try_get::<&PlayerId>(|id| *id) else {
        return;
    };
    impact
        .world
        .get::<&ServerHandle>(|server| server.status(player, tackle_slow()));
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

/// `[WIKI]` "Speed III, Regeneration III, and Strength III, and all your
/// abilities recharge much faster. Lasts 20 seconds."
///
/// The twenty seconds is the ability, and it used to be one burst. Speed and
/// Regeneration are `ClientboundUpdateMobEffect`, which the seam does not carry
/// yet; Strength is [`MeleeBonus`], which it does. What stands in for "abilities
/// recharge much faster" is a Wolf Strike on a beat -- the kit's own lunge,
/// arriving faster than its seven-second cooldown ever allows.
fn frenzy(cast: &Cast<'_>) {
    let now = cast.world.cloned::<&crate::module::damage::MatchClock>().0;
    // Strength III is +3 in vanilla's own table, which is also exactly the gap
    // between Wolf's base 5 and the 8 its own Ravage ceiling reaches -- so a
    // frenzied Wolf starts where a Wolf who has been landing hits ends up.
    cast.caster.set(MeleeBonus {
        flat: FRENZY_BONUS_DAMAGE,
        against: None,
        until: now + ability::ULTIMATE_SECONDS,
    });

    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, FRENZY_INTERVAL, wolf_strike),
    );
}

/// `[WIKI]` Strength III, which vanilla scores at +3 damage.
pub const FRENZY_BONUS_DAMAGE: f32 = RAVAGE_MAX_DAMAGE - 5.0;

/// `[APPROXIMATED]`. Wolf Strike's own cooldown is seven seconds; "much faster"
/// is read here as a lunge every two.
const FRENZY_INTERVAL: f32 = 2.0;
