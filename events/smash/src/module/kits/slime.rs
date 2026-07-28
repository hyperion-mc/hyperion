//! Slime: the resource kit.
//!
//! Slime is the kit where the experience bar is the whole game. Slime Rocket
//! spends it to launch a piece of yourself, and the piece is bigger and hits
//! harder the longer you charged; the bar also *is* your hitbox, so spending it
//! makes you smaller and harder to hit while leaving you with nothing to spend.
//! Slime Slam is the aggressive option and it hurts you too — a quarter of the
//! damage and the knockback come straight back at you.
//!
//! Numbers from the `SMASH_KITS` spreadsheet dump.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        ability::{Cast, Observable, splash_at},
        damage::{DamageKind, Damaged, hurt},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        knockback::Knockback,
        player::Energy,
    },
    server::Cue,
};

/// `Energy Per Tick = 0.004` on a 49 ms tick.
pub const ENERGY_REGEN_PER_SECOND: f32 = 0.0816;

/// `Max Energy Time = 3` seconds of charge, drained at `0.01` per tick.
pub const MAX_CHARGE_SECONDS: f32 = 3.0;

/// Slime Slam recoils a quarter of what it deals back onto the caster.
pub const SLAM_RECOIL_FRACTION: f32 = 0.25;

#[derive(Component)]
pub struct Slime;

impl Module for Slime {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Slime");

        kit::define(world, "Slime", KitStats {
            melee_damage: 6.0,
            armor: 8.0,
            knockback_taken: 1.75,
            regen: 0.35,
            hunger_interval: 7.0,
            jump_power: 1.2,
            energy: Some((1.0, ENERGY_REGEN_PER_SECOND)),
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.slime.squish",
            hurt: "minecraft:entity.slime.hurt",
            death: "minecraft:entity.slime.death",
        })
        .skin(crate::kit_skin!("slime"))
        .blurb("Spend yourself to send enemies flying, and shrink while you do it.")
        .mob("minecraft:slime")
        .ability(AbilitySpec {
            name: "Slime Rocket",
            sound: "minecraft:entity.slime.squish",
            item: "minecraft:iron_sword",
            description: "Hold to grow a rocket out of yourself. Release to launch it.",
            cooldown: 6.0,
            charge_time: Some(MAX_CHARGE_SECONDS),
            // A third of the bar minimum; a full charge costs the lot.
            energy_cost: Some(0.33),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: slime_rocket,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Slime Slam",
            sound: "minecraft:entity.slime.attack",
            item: "minecraft:iron_axe",
            description: "Throw yourself at someone. You take a quarter of it back.",
            cooldown: 6.0,
            // The recoil is a real launch on the caster, not a rounding
            // artefact: a quarter of the hit comes back the other way, which is
            // what makes the ability a commitment.
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: slime_slam,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Giga Slime",
            sound: "minecraft:entity.slime.jump",
            item: "minecraft:nether_star",
            description: "Become enormous and untouchable. Everything near you dies.",
            cooldown: 19.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: giga_slime,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// Charge maps to slime size, and size to damage: `3 + 3 * size`.
///
/// The size is `max(1, floor(charge_seconds))`, so a tap and a one-second hold
/// are the same rocket and the third second is the only one that reaches size
/// three. Charging past that does nothing, which is the kit's skill floor.
#[must_use]
pub fn rocket_size(charge: f32) -> u32 {
    // Mineplex's `max(1, floor(chargeSeconds))` with the charge capped at three
    // seconds. Floor, not round: two and a half seconds is still a size-two
    // rocket, and that hard edge at each whole second is the kit's timing test.
    let seconds = charge.clamp(0.0, 1.0) * MAX_CHARGE_SECONDS;
    if seconds >= 3.0 {
        3
    } else if seconds >= 2.0 {
        2
    } else {
        1
    }
}

#[must_use]
pub const fn rocket_damage(size: u32) -> f32 {
    3.0f32.mul_add(size as f32, 3.0)
}

fn slime_rocket(cast: &Cast<'_>) {
    let size = rocket_size(cast.charge);
    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * (2.0 + size as f32);

    splash_at(
        cast,
        ahead,
        1.0 + size as f32,
        rocket_damage(size),
        cast.charge.mul_add(1.4, 1.0),
    );
    cast.server.cue(ahead, Cue::Explosion);
}

/// Deals 7 at a 2.0 multiplier, and hands a quarter of both back to the caster
/// in the opposite direction.
fn slime_slam(cast: &Cast<'_>) {
    const DAMAGE: f32 = 7.0;
    const KNOCKBACK: f32 = 2.0;

    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * 3.0;
    splash_at(cast, ahead, 2.0, DAMAGE, KNOCKBACK);

    hurt(cast.caster, Damaged {
        attacker: None,
        amount: DAMAGE * SLAM_RECOIL_FRACTION,
        knockback: Knockback::from(ahead).times(KNOCKBACK * SLAM_RECOIL_FRACTION),
        kind: DamageKind::Ability,
    });
}

/// Full damage immunity both ways in Mineplex; here it is the contact damage
/// that matters, applied around a body three blocks tall.
fn giga_slime(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0 + Vec3::Y * 3.0, 5.0, 8.0, 2.0);

    // Growing back to full is part of the ultimate: the bar is the hitbox.
    cast.caster.get::<&mut Energy>(|energy| {
        energy.current = energy.max;
    });
}
