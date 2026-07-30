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
use hyperion::simulation::entity_kind::EntityKind;

use crate::module::{
    ability::{self, Cast, Observable, splash},
    damage::DamageKind,
    effect::{self, Affliction, Shows},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    player::Health,
    projectile::{Flight, Impact, Payload, Visual, fire},
    visuals,
};

/// What a needle leaves behind.
///
/// `[APPROXIMATED]`. The wiki says only that Needler poisons and that armour
/// does not stop it; vanilla Poison I is one point every 25 ticks, which is
/// where the interval comes from. Six seconds is the wiki's "lingers", pinned
/// to a number so the gate has something to wait for.
pub const POISON_SECONDS: f32 = 6.0;
pub const POISON_PER_TICK: f32 = 1.0;
pub const POISON_INTERVAL: f32 = 1.25;

const POISONED: Shows = Shows {
    effect: visuals::venom,
    sound: "minecraft:entity.player.hurt_sweet_berry_bush",
};

/// How much of a hit Spiders Nest gives back to the caster, as a fraction of
/// the damage it dealt.
///
/// `[APPROXIMATED]`. The wiki gives the ultimate as "everything you hit heals
/// you" and no figure; a half is enough to be worth using the dome for and not
/// enough to make a Spider unkillable inside it.
pub const NEST_LIFESTEAL: f32 = 0.5;

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
        .sounds(KitSounds {
            select: "minecraft:entity.spider.ambient",
            hurt: "minecraft:entity.spider.hurt",
            death: "minecraft:entity.spider.death",
        })
        .cost(0)
        .skin(crate::kit_skin!("spider"))
        .blurb("Fast, fragile and everywhere at once. Leap where you look.")
        .mob("minecraft:spider")
        .ability(AbilitySpec {
            name: "Needler",
            sound: "minecraft:entity.bee.sting",
            item: "minecraft:iron_sword",
            description: "Spray six needles. They poison, which armour does not stop.",
            cooldown: 6.0,
            charge_time: Some(1.0),
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::AfflictsTarget,
            ],
            activate: needler,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Spin Web",
            sound: "minecraft:block.cobweb.place",
            item: "minecraft:iron_axe",
            description: "Launch forward, trailing web. Mostly a way back onto the map.",
            cooldown: 8.0,
            proves: &[Observable::LaunchesCaster],
            activate: spin_web,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Spiders Nest",
            sound: "minecraft:entity.spider.ambient",
            item: "minecraft:nether_star",
            description: "Twenty seconds inside a dome of web. Everything you hit heals you.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::HealsCaster,
                Observable::Sustains,
            ],
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
            // `[APPROXIMATED]`: a needle has no entity; a spectral arrow reads as
            // a thin magic dart.
            Visual(EntityKind::SpectralArrow),
            Flight {
                position: cast.position.0,
                velocity: direction * 34.0,
                gravity: 12.0,
                seconds_left: 1.5,
                radius: 0.6,
            },
            Payload::new(1.5, 0.35).then(envenom),
        );
    }
}

/// The half of Needler the tooltip promises and the impact alone cannot give:
/// the needle goes in, and the poison keeps working afterwards.
///
/// `DamageKind::Environment` is what "armour does not stop it" means; it is the
/// same mechanism Blaze's burn uses and the same reason.
fn envenom(impact: &Impact<'_>) {
    let Some(blame) = effect::Blame::impact(impact) else {
        return;
    };
    effect::afflict(
        impact.world,
        impact.victim,
        blame,
        Affliction::over_time(
            POISON_SECONDS,
            POISON_PER_TICK,
            POISON_INTERVAL,
            DamageKind::Environment,
            POISONED,
        ),
    );
}

/// How much floor the web left behind covers.
///
/// `[APPROXIMATED]`, and it had to be invented: Spin Web affects nobody but the
/// caster, so unlike every other area here there is no gameplay radius to draw
/// at. This is sized to read as a patch of floor rather than a puff, and it
/// promises nothing, because standing in it does nothing.
const WEB_RADIUS: f32 = 2.5;

/// A leap that leaves web behind. `[APPROXIMATED]`: the recovery distance is
/// tuned to "decent horizontal and vertical", not to a published number.
fn spin_web(cast: &Cast<'_>) {
    let launch = cast.facing.0.normalize_or_zero() * 1.4 + Vec3::Y * 0.45;
    cast.server.add_velocity(cast.player, launch);
    // At the spot the Spider left, which is the whole of what the ability did:
    // the web is the trail, and the leap carries them off it immediately.
    cast.server
        .particles(visuals::web(cast.position.0, WEB_RADIUS));
}

/// `[APPROXIMATED]`: the dome traps, which needs block writes the game half
/// cannot make. The damage and the one-second recharge are the wiki's, and so
/// is "everything you hit heals you".
///
/// A dome is somewhere you stand for a while. The ability is now the twenty
/// seconds, with the same pulse -- and the lifesteal on every beat, which is
/// what makes a Spider in their own nest the problem the wiki describes.
fn spiders_nest(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, NEST_INTERVAL, nest_pulse),
    );
}

/// `[WIKI]` "the one-second recharge", which is the beat.
const NEST_INTERVAL: f32 = 1.0;

fn nest_pulse(cast: &Cast<'_>) {
    const DAMAGE: f32 = 4.0;

    // Counted from the victims the blast returned rather than from a second
    // query, so the heal is exactly as large as the damage that was dealt: a
    // Spider standing alone in their own dome heals for nothing.
    let hits = splash(cast, 6.0, DAMAGE, 0.8).len();
    if hits == 0 {
        return;
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a splash cannot return more victims than there are players in a match"
    )]
    let stolen = DAMAGE * NEST_LIFESTEAL * hits as f32;
    let (current, max) = cast.caster.get::<&mut Health>(|health| {
        health.heal(stolen);
        (health.current, health.max)
    });
    cast.server.set_health(cast.player, current, max);
}
