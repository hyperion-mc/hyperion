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
use hyperion::simulation::entity_kind::EntityKind;

use crate::{
    module::{
        ability::{self, Cast, Observable, splash_at},
        damage::MatchClock,
        effect::{self, Affliction},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        player::{Player, Position},
        projectile::{Flight, Impact, Payload, Visual, fire},
        visuals,
    },
    server::{PlayerId, ServerHandle},
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
            select: "minecraft:entity.guardian.ambient",
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
            description: "Twenty seconds of water. Everything in it goes where it goes.",
            cooldown: 20.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
            activate: tidal_wave,
            ..AbilitySpec::DEFAULT
        })
        .register();

        // The mark lapses on its own, and lapsing costs the victim three more
        // damage whether or not the Guardian is still nearby.
        world
            .system_named::<(&Marked, &PlayerId)>("laser_expiry")
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
        // `[APPROXIMATED]`: a water shard has no entity; a spectral arrow is
        // the closest glinting bolt that always renders.
        Visual(EntityKind::SpectralArrow),
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
    // Where the axe leaves the caster. The shard renders as a spectral arrow
    // once it is airborne, so the water thrown off it on the way out is the
    // only part of the throw that says which kit threw it.
    cast.server.particles(visuals::spray(cast.position.0));
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
    cast.server.particles(visuals::blast(cast.position.0));
}

fn target_laser(cast: &Cast<'_>) {
    // First, above the target lookup, because the lookup can fail: press the
    // button with nobody inside `LASER_RANGE` and the `else { return }` below
    // spends the ability and leaves. Anything drawn after that point is drawn
    // only when the ability worked, which is a visual whose presence depends
    // on the game state rather than on the press.
    cast.server.particles(visuals::laser_eye(cast.position.0));

    let now = cast.world.get::<&MatchClock>(|clock| clock.0);
    let caster = cast.caster.id();
    let mut nearest: Option<(f32, Entity, Vec3)> = None;
    cast.world
        .query::<&Position>()
        .with(Player::id())
        .build()
        .each_entity(|entity, position| {
            if entity.id() == caster {
                return;
            }
            let distance = position.0.distance(cast.position.0);
            if distance <= LASER_RANGE && nearest.is_none_or(|(best, ..)| distance < best) {
                nearest = Some((distance, entity.id(), position.0));
            }
        });

    let Some((_, victim, marked_at)) = nearest else {
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

    // The beam on top of the flare above, and then again every beat until the
    // mark lapses. Everything the ability does is invisible -- a component on
    // the caster and a number on somebody else's incoming damage -- so without
    // this the one ability the kit is named for is the one nobody can see. A
    // line between the two players rather than a second puff at the caster,
    // because *who* it landed on is the whole of what the ability says, and it
    // is the only part the flare cannot carry.
    //
    // Off the position the query above already read, not off `Marked`:
    // `activate` runs inside an observer, so the `set` a few lines up is
    // queued until the frame ends and reading it back here would find nothing
    // on a first cast and the previous victim on a second. `laser_beam` is a
    // beat, which is a frame later at the earliest, so there it is committed.
    //
    // What a line of particles gives up against vanilla's guardian beam is
    // continuity: a real beam is one unbroken thing and this one blinks twice
    // a second, because the seam cannot spawn the beam entity that draws the
    // unbroken version.
    cast.server
        .particles(visuals::mark_beam(cast.position.0, marked_at));
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(LASER_SECONDS, LASER_BEAM_INTERVAL, laser_beam),
    );
}

/// `[APPROXIMATED]`. Fast enough that the mark never vanishes for long enough
/// to be missed, slow enough that eight seconds of it is sixteen lines rather
/// than a line every frame.
const LASER_BEAM_INTERVAL: f32 = 0.5;

/// Redraw the mark between wherever the two of them have got to.
///
/// Reads [`Marked`] back off the caster rather than closing over the victim,
/// for two reasons: both ends move, and the mark can end early, in which case
/// the component is gone and this draws nothing.
fn laser_beam(cast: &Cast<'_>) {
    let Some(marked) = cast.caster.try_get::<&Marked>(|marked| *marked) else {
        return;
    };
    let victim = cast.world.entity_from_id(marked.victim);
    if !victim.is_alive() {
        return;
    }
    let Some(at) = victim.try_get::<&Position>(|position| position.0) else {
        return;
    };
    cast.server
        .particles(visuals::mark_beam(cast.position.0, at));
}

/// `[APPROXIMATED]` throughout; the wiki describes the ultimate and gives no
/// figures.
///
/// A wave is a thing that keeps arriving. One splash was a slap.
fn tidal_wave(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, TIDE_INTERVAL, tide),
    );
}

const TIDE_INTERVAL: f32 = 1.5;

/// Per wave, and there are thirteen.
const TIDE_DAMAGE: f32 = 3.0;

/// `[APPROXIMATED]`, as the rest of the ultimate is. Named rather than written
/// twice, so the ring a player backs away from is the ring that hits them.
const TIDE_RADIUS: f32 = 10.0;

fn tide(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, TIDE_RADIUS, TIDE_DAMAGE, 2.2);
    // On the beat rather than at the cast: the ultimate is thirteen waves over
    // twenty seconds, and one ring at the start leaves the other twelve unseen.
    cast.server
        .particles(visuals::tide(cast.position.0, TIDE_RADIUS));
}
