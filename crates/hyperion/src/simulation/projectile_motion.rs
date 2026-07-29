//! Vanilla's per-tick integration for a projectile, as data.
//!
//! Every number and every ordering here is transcribed from the pinned server
//! jar, and every one of them is checked against it: `crates/hyperion/tests/
//! differential.rs` replays the recorded flight of each kind tick by tick, and
//! `nix/differential.nix` re-records those traces from the jar so a version
//! bump cannot leave this table quietly wrong. See `docs/differential-testing.md`.
//!
//! The surprising part is that vanilla does not have one projectile
//! integrator, it has two, and they differ in more than their constants:
//!
//! ```text
//! AbstractArrow.tick()          ThrowableProjectile.tick()
//!   pos += v                      v.y -= gravity
//!   v   *= drag                   v   *= drag
//!   v.y -= gravity                pos += v
//! ```
//!
//! An arrow therefore travels its full launch speed on its first tick and only
//! then loses any of it, while a snowball is already slowed and falling before
//! it moves at all. Sharing one integrator between them is wrong by about a
//! tenth of a block on the first tick and it never converges.

use flecs_ecs::prelude::*;
use glam::Vec3;

use super::entity_kind::EntityKind;

/// Where the position update sits relative to the velocity update.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MotionOrder {
    /// Position first, then drag, then gravity: `AbstractArrow.tick`.
    MoveThenDecay,
    /// Gravity first, then drag, then position: `ThrowableProjectile.tick`.
    DecayThenMove,
}

/// One tick of vanilla projectile motion.
///
/// A per-instance component, not only a per-kind lookup: the kind's entry in
/// [`SIMULATED`] is the vanilla default an `OnAdd` observer seeds, and a game
/// module may override it on a single projectile (a hook with no gravity, a
/// heavier lob) without inventing a new entity kind.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct ProjectileMotion {
    /// The velocity is multiplied by this every tick, out of water.
    ///
    /// `AbstractArrow.getAirDrag` and `ThrowableProjectile.getAirDrag` both
    /// return the `float` 0.99, and vanilla widens that to a `double` before
    /// multiplying, so the value it actually applies is 0.990000009536743.
    /// Stored as an `f32` here for the same reason: hyperion's velocities are
    /// `f32`, so the widening never happens and the constant is the one the
    /// jar's own literal denotes.
    pub drag: f32,
    /// Subtracted from the vertical velocity every tick.
    ///
    /// `Entity.applyGravity` reads `getDefaultGravity`, which is 0.05 for
    /// arrows and 0.03 for anything thrown.
    pub gravity: f32,
    /// Which of the two tick shapes above this kind uses.
    pub order: MotionOrder,
}

impl ProjectileMotion {
    /// Advances one tick in place.
    ///
    /// Both branches are vanilla's statements in vanilla's order. Reordering
    /// them to share code would change the answer, which is the whole point of
    /// the enum.
    pub fn step(self, position: &mut Vec3, velocity: &mut Vec3) {
        match self.order {
            MotionOrder::MoveThenDecay => {
                *position += *velocity;
                *velocity *= self.drag;
                velocity.y -= self.gravity;
            }
            MotionOrder::DecayThenMove => {
                velocity.y -= self.gravity;
                *velocity *= self.drag;
                *position += *velocity;
            }
        }
    }
}

/// The yaw and pitch, in degrees, a projectile stores while travelling along
/// `velocity`.
///
/// Vanilla derives an in-flight projectile's orientation from its velocity
/// every tick, in `Projectile.updateRotation` and `AbstractArrow.tick` alike,
/// as `yaw = atan2(dx, dz)` and `pitch = atan2(dy, horizontalDistance)`. That
/// is the projectile-entity sign convention, and it is not the one a shooter's
/// own look yaw uses: the look direction inverts to `atan2(-dx, dz)`, so an
/// arrow loosed due west stores yaw -90 where the player who fired it reads
/// +90. Handing the client the player's yaw instead of this is the wrong
/// heading a bystander sees, an arrow that renders mirrored across its own line
/// of flight.
///
/// `f32::atan2` where vanilla calls `Mth.atan2`, a table approximation the two
/// agree with to well under a degree; `crates/hyperion/tests/differential.rs`
/// holds the difference under each scenario's rotation tolerance against a
/// recording of the real server.
#[must_use]
pub fn look_angles(velocity: Vec3) -> (f32, f32) {
    let horizontal = velocity.x.hypot(velocity.z);
    let yaw = velocity.x.atan2(velocity.z).to_degrees();
    let pitch = velocity.y.atan2(horizontal).to_degrees();
    (yaw, pitch)
}

/// A standing player's eye height, in blocks (`Player.getStandingEyeHeight`).
///
/// A projectile leaves the eye, not the feet the entity position tracks, so
/// the vanilla bow and every ability that fires "from where you are looking"
/// both add this before launching. Fire from the feet and the shot renders as
/// one loosed from the stomach.
pub const EYE_HEIGHT: f32 = 1.62;

/// How far in front of the eye a launched projectile starts, in blocks. Vanilla
/// nocks the arrow a half block along the look so it clears the shooter's own
/// hitbox on the first tick.
pub const MUZZLE_OFFSET: f32 = 0.5;

/// The point a projectile launches from: the shooter's eye, a half block along
/// `direction`. `feet` is the tracked entity [`super::Position`]; `direction`
/// is the unit look vector from [`super::get_direction_from_rotation`]. Both
/// events call this so the muzzle is written down once.
#[must_use]
pub fn muzzle(feet: Vec3, direction: Vec3) -> Vec3 {
    Vec3::new(feet.x, feet.y + EYE_HEIGHT, feet.z) + direction * MUZZLE_OFFSET
}

/// Blocks per tick a fully drawn bow gives its arrow.
///
/// `BowItem.releaseUsing` passes `getPowerForTime(...) * 3.0` to the arrow and
/// the curve saturates at 1.0, so a full draw is exactly this.
pub const MAX_ARROW_SPEED: f32 = 3.0;

/// Vanilla's `BowItem.getPowerForTime`, keyed on the draw as a fraction of a
/// full draw rather than a raw tick count.
///
/// `power = (f*f + f*2) / 3`, clamped to 1.0, where `f` is `drawTicks / 20`.
/// Returned as a fraction so the caller's `* MAX_ARROW_SPEED` lands on exactly
/// vanilla's maximum. Both events call this rather than transcribe the curve a
/// second time: bedwars keys `f` on the held `Duration`, smash on an ability's
/// 0..1 charge.
#[must_use]
#[expect(
    clippy::suboptimal_flops,
    reason = "mul_add is a fused multiply-add: one rounding where Java does two. getPowerForTime \
              evaluates `(f * f + f * 2.0F) / 3.0F` as separate float operations, and this \
              function exists to give the same answer it does, so the two roundings are the \
              behaviour rather than an oversight"
)]
pub fn bow_power(draw_fraction: f32) -> f32 {
    let f = draw_fraction;
    ((f * f + f * 2.0) / 3.0).min(1.0)
}

/// One tick of vanilla's rotation smoothing: `Projectile.lerpRotation`.
///
/// Slides `current` by whole turns into the half-open window
/// `[target - 180, target + 180)` so the short way round is always taken, then
/// moves it a fifth of the way to `target`. Seeded exactly by [`look_angles`]
/// at launch, so a heading that is not changing stays put and one that is eases
/// toward its new value over five ticks rather than snapping.
#[must_use]
#[expect(
    clippy::suboptimal_flops,
    reason = "vanilla's Mth.lerp is `a + t * (b - a)` as separate float operations, not a fused \
              multiply-add; this file exists to give the same answer, so the two roundings are \
              the behaviour rather than an oversight"
)]
#[expect(
    clippy::while_float,
    reason = "vanilla's `lerpRotation` is literally these two `while` loops; each step moves the \
              angle by exactly 360 toward a fixed bound, so it terminates in at most a turn or \
              two, and matching its form is the point"
)]
pub fn lerp_rotation(mut current: f32, target: f32) -> f32 {
    while target - current < -180.0 {
        current -= 360.0;
    }
    while target - current >= 180.0 {
        current += 360.0;
    }
    current + 0.2 * (target - current)
}

/// Everything that reaches `AbstractArrow.tick`.
const ARROW: ProjectileMotion = ProjectileMotion {
    drag: 0.99,
    gravity: 0.05,
    order: MotionOrder::MoveThenDecay,
};

/// Everything that reaches `ThrowableProjectile.tick`.
const THROWN: ProjectileMotion = ProjectileMotion {
    drag: 0.99,
    gravity: 0.03,
    order: MotionOrder::DecayThenMove,
};

/// Every kind whose flight this server integrates, and how.
///
/// One table rather than a match, because the differential test walks it to
/// turn the `minecraft:snowball` in a scenario file into a kind. A second list
/// would be a second thing to forget.
pub const SIMULATED: &[(EntityKind, ProjectileMotion)] = &[
    (EntityKind::Arrow, ARROW),
    (EntityKind::SpectralArrow, ARROW),
    (EntityKind::Trident, ARROW),
    (EntityKind::Snowball, THROWN),
    (EntityKind::Egg, THROWN),
    (EntityKind::EnderPearl, THROWN),
    (EntityKind::ExperienceBottle, THROWN),
];

impl EntityKind {
    /// How this kind moves under its own momentum, or `None` if the server
    /// does not simulate its flight.
    ///
    /// `None` rather than a default, because a default would silently give
    /// every unlisted entity an arrow's physics: a kind that is missing from
    /// [`SIMULATED`] is a kind nobody has checked against the jar, and
    /// standing still is a far more obvious wrong answer than falling at
    /// almost the right rate.
    #[must_use]
    pub fn projectile_motion(self) -> Option<ProjectileMotion> {
        SIMULATED
            .iter()
            .find(|(kind, _)| *kind == self)
            .map(|(_, motion)| *motion)
    }
}

#[cfg(test)]
mod tests {
    use super::{MotionOrder, SIMULATED};

    /// Every simulated kind must be something a client can be told about,
    /// since an entity nobody can see is not worth integrating.
    #[test]
    fn every_simulated_kind_has_an_entity_type() {
        for (kind, _) in SIMULATED {
            assert!(
                kind.entity_type().is_some(),
                "{kind:?} is simulated but has no entity type in this protocol version"
            );
        }
    }

    /// The two orders are the reason this module exists, so a table that
    /// collapsed onto one of them would have lost the distinction.
    #[test]
    fn both_orders_are_used() {
        assert!(
            SIMULATED
                .iter()
                .any(|(_, m)| m.order == MotionOrder::MoveThenDecay)
        );
        assert!(
            SIMULATED
                .iter()
                .any(|(_, m)| m.order == MotionOrder::DecayThenMove)
        );
    }
}
