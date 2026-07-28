//! What the recurring moments of the game look like.
//!
//! An ability going off, a teleport landing, a player dying, a burn ticking, a
//! poison ticking. Each is one moment that happens in a dozen places and should
//! look the same in all of them, so each is written once, here.
//!
//! This is not the old `Cue` enum under another name. A `Cue` was a closed set
//! and the *only* thing an ability could ask for, which is why Bone Explosion,
//! Water Splash and Fish Flurry were all `Cue::Explosion` and all drew the same
//! grey puff. These are plain functions returning a [`Particles`], so a kit
//! that wants bones rather than a blast composes its own and does not have to
//! widen anything:
//!
//! ```ignore
//! cast.server.particles(
//!     Particles::sphere(Particle::Item { item_stack: bone }, at, 2.0)
//!         .points(30)
//!         .speed(0.1),
//! );
//! ```
//!
//! `[INFERRED]` throughout. Mineplex's own particle choices are not in the
//! leaked source, which loaded them from the same spreadsheet as everything
//! else, so these are read off what vanilla draws for the same events.

use glam::Vec3;

use crate::server::{Argb, Particle, Particles};

/// Half-width of the box a point effect scatters its particles through.
///
/// Small enough to read as one thing happening in one place rather than as
/// weather.
const SPREAD: f32 = 0.4;

/// How fast a scattered particle drifts. Nearly still: these mark a place, and
/// a puff that flies apart stops marking it.
const DRIFT: f32 = 0.02;

/// Roughly chest height on a standing player, where a status effect reads best.
const CHEST: f32 = 0.9;

/// An ability going off here.
///
/// The default for anything with no look of its own yet, which is most of
/// them, and the one to stop reaching for as soon as a kit has something to
/// say.
pub const fn blast(at: Vec3) -> Particles {
    Particles::burst(Particle::Explosion, at)
        .count(40)
        .offset(Vec3::splat(SPREAD))
        .speed(0.5)
        // Something that just happened to a player is worth seeing from
        // further out than the client's usual particle radius.
        .long_distance(true)
}

/// Somebody arriving somewhere they were not.
///
/// A sphere rather than a puff, because a teleport has a shape: the eye reads
/// a shell as an arrival and a scatter as damage.
pub fn teleport(at: Vec3) -> Particles {
    Particles::sphere(Particle::Portal, at + Vec3::Y, 1.0)
        .points(40)
        .count(2)
        .speed(0.05)
        .long_distance(true)
}

/// A player dying here.
pub fn death(at: Vec3) -> Particles {
    Particles::burst(Particle::Cloud, at + Vec3::Y)
        .count(40)
        .offset(Vec3::new(SPREAD, 0.8, SPREAD))
        .speed(0.1)
        .long_distance(true)
}

/// One tick of something burning a player.
///
/// `minecraft:flame`, which is what vanilla draws on a burning entity. This was
/// pinned to `crit` for as long as the protocol layer could spell only five
/// particles.
pub fn burn(at: Vec3) -> Particles {
    Particles::burst(Particle::Flame, at + Vec3::Y * CHEST)
        .count(12)
        .offset(Vec3::new(0.3, 0.5, 0.3))
        .speed(DRIFT)
}

/// One tick of something poisoning a player.
///
/// `minecraft:entity_effect` in vanilla's own poison green, which is what a
/// potion effect draws. Distinct from [`burn`] by its picture and by nothing
/// else, which is the whole point: both take a point of health a second off
/// somebody standing still, and a player who cannot tell them apart cannot tell
/// a Blaze from a Spider.
pub fn venom(at: Vec3) -> Particles {
    Particles::burst(
        Particle::EntityEffect {
            // Vanilla's own poison colour, which is what `entity_effect`
            // is drawn in when a `minecraft:poison` instance renders:
            // `MobEffects.POISON` carries `0x4E9331`. Taken from the effect
            // rather than picked, so the haze around a poisoned player is the
            // green a player already reads as poison rather than an arbitrary
            // green.
            color: Argb::opaque(0x4E, 0x93, 0x31),
        },
        at + Vec3::Y * CHEST,
    )
    .count(12)
    .offset(Vec3::new(0.3, 0.5, 0.3))
    .speed(DRIFT)
}
