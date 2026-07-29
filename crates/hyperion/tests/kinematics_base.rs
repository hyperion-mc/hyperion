//! Dev-profile guard for the kinematics registration base (ENG-11000's class).
//!
//! The flecs module convention (root `CLAUDE.md`) says a consumer can import a
//! registration module on its own and get its components registered, with no
//! behavior and no need to boot the whole engine. The projectile physics relies
//! on exactly this: it imports [`KinematicsComponentsModule`] to get `Position`
//! and `Velocity` without pulling in the simulation's observers.
//!
//! `Position`/`Velocity` register their reflection through the glam `Vec3` meta,
//! so a naive standalone registration would abort a dev build with
//! `ECS_INVALID_OPERATION` the first time the meta is registered without `Vec3`
//! present. `KinematicsComponentsModule` imports `ReflectionComponentsModule`
//! to close that gap; this test is the proof, in the dev build `cargo test`
//! produces. It fails (aborts) if the reflection import is dropped.

use flecs_ecs::{core::World, prelude::*};
use hyperion::simulation::{KinematicsComponentsModule, Position, Velocity};
use serial_test::serial;

#[test]
#[serial]
#[expect(
    clippy::float_cmp,
    reason = "the assertions round-trip the exact literals just set through the bare world; an \
              epsilon would weaken the claim, which is that the component stored and returned the \
              same bits rather than that it stored something close"
)]
fn kinematics_base_is_usable_after_standalone_import() {
    // A bare world: no HyperionCore, no SimModule, nothing but the kinematics
    // registration module. This is the smash mock / projectile-physics shape.
    let world = World::new();
    world.import::<KinematicsComponentsModule>();

    // Using the components in a dev build is the guard: a `set`/`get` of an
    // unregistered component aborts under `flecs_manual_registration`, and the
    // `.meta()` over `Vec3` would abort at import time without the reflection
    // base. Both are exercised here.
    let entity = world.entity();
    entity.set(Position::new(1.0, 2.0, 3.0));
    entity.set(Velocity::new(0.0, -1.0, 0.0));

    let (px, vy) = entity.get::<(&Position, &Velocity)>(|(p, v)| (p.x, v.0.y));
    assert_eq!(px, 1.0, "Position should round-trip through the bare world");
    assert_eq!(
        vy, -1.0,
        "Velocity should round-trip through the bare world"
    );
}
