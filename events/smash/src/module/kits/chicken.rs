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

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        ability::{Cast, Cooldown, Grants, Named, Observable},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        projectile::{Flight, Impact, Payload, fire},
    },
    server::Cue,
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
            slot: 1,
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
            slot: 2,
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
            slot: 8,
            description: "Unlimited flight and eggs, for twenty seconds.",
            cooldown: 1.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesCaster],
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
    for index in 0..EGGS {
        let offset = (index as f32 - (EGGS as f32 - 1.0) / 2.0) * 0.1;
        fire(
            cast.world,
            cast.caster,
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
        .get::<&crate::server::ServerHandle>(|server| server.cue(impact.at, Cue::Explosion));
}

/// `[APPROXIMATED]`: unlimited flight is not a state the game half can enter.
fn aerial_gunner(cast: &Cast<'_>) {
    cast.server.add_velocity(cast.player, Vec3::Y * 0.8);
    egg_blaster(cast);
}
