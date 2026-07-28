//! Pushing things around.
//!
//! There is no impulse component and no queue here. [`Velocity`] already
//! exists, and `sync_entity_state` already turns a non-zero one into exactly
//! one `SetEntityMotion` and zeroes it again, so a second accumulator would be
//! a second thing to keep in step with the first. What was missing is the
//! arithmetic: an away-vector with a lift on it, written once instead of at
//! every call site that wants to knock somebody back.
//!
//! # Units
//!
//! Blocks per tick, throughout. Protocol 776 sends velocity through
//! `Vec3.LP_STREAM_CODEC`, a packed fifteen-bits-per-axis quantisation, not
//! the 1/8000-block shorts 1.20.1 used, and that packing lives in the codec.
//! So there is no fixed-point conversion at this layer to get wrong. There is
//! still a rounding step, and [`quantized`] is what a caller or a test uses to
//! ask what the client will actually receive.

use flecs_ecs::core::{EntityView, EntityViewGet};
use glam::Vec3;
use hyperion_minecraft_proto::{packets::play::entity::lp_vec3, types::Vec3 as ProtoVec3};

use crate::simulation::Velocity;

/// Add to an entity's velocity, in blocks per tick.
///
/// Additive rather than absolute, so two sources landing in one tick both
/// count. Writing a component's value is not a structural change, so this is
/// safe from inside a system iterating the same entity; adding [`Velocity`] to
/// an entity that has none is not, and this leaves such an entity alone rather
/// than restructuring its table mid-iteration.
pub fn apply_impulse(entity: EntityView<'_>, delta: Vec3) {
    entity.try_get::<&mut Velocity>(|velocity| velocity.0 += delta);
}

/// Set an entity's velocity outright, in blocks per tick.
///
/// For the cases where the previous velocity is not part of the answer, such
/// as a launch that should feel the same whether or not you were already
/// falling.
pub fn set_velocity(entity: EntityView<'_>, velocity: Vec3) {
    entity.try_get::<&mut Velocity>(|current| current.0 = velocity);
}

/// Knock `entity` away from `origin`.
///
/// `magnitude` is the horizontal push and `lift` the vertical one, both in
/// blocks per tick. The caller owns both numbers: this is deliberately not a
/// second knockback model. Super Smash Mobs scales its own by damage and by
/// missing health in `smash::module::knockback::strength`, and that stays the
/// only place in the game those terms live.
pub fn knock_back(entity: EntityView<'_>, from: Vec3, magnitude: f32, lift: f32) {
    let at = entity.try_get::<&crate::simulation::Position>(|position| **position);
    if let Some(at) = at {
        apply_impulse(entity, knockback_impulse(from, at, magnitude, lift));
    }
}

/// The impulse that pushes something at `target` away from `origin`.
///
/// Horizontal only, plus `lift` straight up. A knockback that took the
/// vertical separation into account would send someone standing on your head
/// straight up and someone below you straight down, which is not what being
/// hit feels like.
#[must_use]
pub fn knockback_impulse(origin: Vec3, target: Vec3, magnitude: f32, lift: f32) -> Vec3 {
    let away = Vec3::new(target.x - origin.x, 0.0, target.z - origin.z);
    // Two things at exactly the same spot have no direction between them, and
    // an arbitrary one is better than a NaN: straight up is the only choice
    // that does not favour a compass direction.
    let away = away.normalize_or(Vec3::ZERO);
    away * magnitude + Vec3::Y * lift
}

/// The velocity the client will actually see, after the wire rounds it.
///
/// `Vec3.LP_STREAM_CODEC` shares one exponent across all three axes and keeps
/// fifteen bits of each, so the error on a small component grows with the
/// largest one. A test asserting that a knockback arrived should compare
/// against this rather than against the value it asked for.
#[must_use]
pub fn quantized(velocity: Vec3) -> Vec3 {
    let packed = lp_vec3::quantize(&ProtoVec3 {
        x: f64::from(velocity.x),
        y: f64::from(velocity.y),
        z: f64::from(velocity.z),
    });
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value went in as an f32 and the codec only rounds it"
    )]
    Vec3::new(packed.x as f32, packed.y as f32, packed.z as f32)
}
