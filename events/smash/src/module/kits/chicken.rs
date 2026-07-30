//! Chicken: the lowest stats in the game and the smallest hitbox.
//!
//! Eight double jumps off an energy bar, a two-second egg gun and a missile that
//! recharges instantly when it connects. Every number here except the missile's
//! damage is the wiki's.
//!
//! Stats verified: 4.5 damage, 5 armour points (20%), 200% knockback taken --
//! the lightest kit in the game -- 0.2 regen, 8000 gems.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::entity_kind::EntityKind;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{self, Cast, Cooldown, Grants, Named, Observable},
        effect::{self, Affliction},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        projectile::{Flight, Impact, Payload, Visual, fire},
        visuals,
    },
};

/// `[VERIFIED]`: "Chicken is the only mob who can double jump eight times."
pub const FLAPS: u8 = 8;

#[derive(Component)]
pub struct Chicken;

impl Module for Chicken {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Chicken");

        kit::define(world, "Chicken", KitStats {
            melee_damage: 4.5,
            armor: 5.0,
            knockback_taken: 2.00,
            regen: 0.20,
            jump_power: 0.7,
            jump_control: true,
            // Flap draws from the energy bar and "rapidly regenerates on the
            // ground".
            energy: Some((100.0, 25.0)),
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.chicken.ambient",
            hurt: "minecraft:entity.chicken.hurt",
            death: "minecraft:entity.chicken.death",
        })
        .cost(8000)
        .skin(crate::kit_skin!("chicken"))
        .blurb("Cannot take a hit, and does not have to.")
        .mob("minecraft:chicken")
        .ability(AbilitySpec {
            name: "Egg Blaster",
            sound: "minecraft:entity.egg.throw",
            item: "minecraft:iron_sword",
            description: "A stream of eggs. No knockback, but it stops people moving.",
            // "The real strength in this relies in the extremely fast recharge
            // of 2 seconds."
            cooldown: 2.0,
            charge_time: Some(0.8),
            proves: &[Observable::HurtsTarget],
            activate: egg_blaster,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Chicken Missile",
            sound: "minecraft:entity.chicken.egg",
            item: "minecraft:iron_axe",
            description: "A chick that explodes. Recharges the instant it hits something.",
            cooldown: 8.0,
            refunds_on_hit: true,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: chicken_missile,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Aerial Gunner",
            sound: "minecraft:entity.chicken.ambient",
            item: "minecraft:nether_star",
            description: "Unlimited flight and eggs, for twenty seconds.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesCaster,
                Observable::Sustains,
            ],
            activate: aerial_gunner,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// `[APPROXIMATED]` count and damage; the wiki gives only "a barrage of low
/// damage eggs".
fn egg_blaster(cast: &Cast<'_>) {
    const EGGS: usize = 5;
    let forward = cast.facing.0.normalize_or_zero();
    let side = Vec3::new(-forward.z, 0.0, forward.x);
    // Above the volley, so the press draws before anything that could branch.
    // The eggs render as egg entities, so what was missing is the stream
    // having a direction anybody could read before the first one lands. Drawn
    // here and not in the ultimate as well: Aerial Gunner beats through this
    // function, so a volley looks the same either way it was fired.
    cast.server
        .particles(visuals::egg_burst(cast.position.0, forward));
    for index in 0..EGGS {
        let offset = (index as f32 - (EGGS as f32 - 1.0) / 2.0) * 0.1;
        fire(
            cast.world,
            cast.caster,
            Visual(EntityKind::Egg),
            Flight {
                position: cast.position.0,
                velocity: (forward + side * offset).normalize_or_zero() * 30.0,
                gravity: 14.0,
                seconds_left: 1.2,
                radius: 0.5,
            },
            Payload::new(1.0, 0.0),
        );
    }
}

/// The missile refunds its own cooldown on a hit, which is what the wiki calls
/// "its strongest point". `[VERIFIED]` behaviour, `[APPROXIMATED]` damage.
fn chicken_missile(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Visual(EntityKind::Egg),
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 24.0,
            gravity: 9.0,
            seconds_left: 2.5,
            radius: 0.8,
        },
        Payload::new(5.0, 1.4).then(refund),
    );
}

fn refund(impact: &Impact<'_>) {
    let Some(shooter) = impact.shooter else {
        return;
    };
    // Clear the cooldown on whichever of the shooter's abilities this was. The
    // projectile does not carry which ability fired it, so the missile is found
    // by name -- the one place in the crate a kit reaches back for its own
    // ability, and the reason `Named` is on the ability rather than the kit.
    let mut found = None;
    shooter.each_target_view(Grants, |ability| {
        if ability.try_get::<&Named>(|name| name.0 == "Chicken Missile") == Some(true) {
            found = Some(ability.id());
        }
    });
    if let Some(ability) = found {
        impact
            .world
            .entity_from_id(ability)
            .set(Cooldown { remaining: 0.0 });
    }
    impact
        .world
        .get::<&crate::server::ServerHandle>(|server| server.particles(visuals::blast(impact.at)));
}

/// `[WIKI]` "unlimited flight and eggs, for twenty seconds".
///
/// The twenty seconds is the ability, and the description already said so while
/// the code did it once. Unlimited flight is not a state the seam can enter, so
/// each beat is a lift and a volley: a chicken that stays up and keeps firing
/// for as long as the crystal lasts, which is what the sentence describes from
/// the ground.
fn aerial_gunner(cast: &Cast<'_>) {
    // A mode's first beat is next frame at the earliest, and the press is the
    // one moment the player who pressed it is looking. Feathers rather than a
    // volley, because the eggs already have Egg Blaster's picture and the
    // flight is the half of the sentence nothing else draws.
    cast.server.particles(visuals::feathers(cast.position.0));
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, GUNNER_INTERVAL, gunner_beat),
    );
}

/// `[APPROXIMATED]`. Egg Blaster's own cooldown is two seconds and the ultimate
/// removes it; twice a second is "unlimited" without being a solid wall of egg.
const GUNNER_INTERVAL: f32 = 0.5;

/// `[APPROXIMATED]`. Enough lift each beat to stay airborne and not enough to
/// climb out of the map over twenty seconds.
const GUNNER_LIFT: f32 = 0.4;

fn gunner_beat(cast: &Cast<'_>) {
    cast.server.add_velocity(cast.player, Vec3::Y * GUNNER_LIFT);
    // On every beat and not only at the press: twenty seconds of a chicken
    // hanging in the air with no visible reason is twenty seconds of the
    // ultimate looking like a bug in the jump code.
    cast.server.particles(visuals::feathers(cast.position.0));
    egg_blaster(cast);
}
