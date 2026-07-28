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
#[derive(Debug, Copy, Clone, PartialEq)]
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
