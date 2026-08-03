//! An arrow that has landed stays landed.
//!
//! `AbstractArrow.tick` opens with two statements the integrator used to have
//! neither of: the shake counts down (`AbstractArrow.java:178-180`), and an
//! arrow already in the ground returns before it moves, drags or falls
//! (lines 184-200). Without the second one a stopped arrow is only stopped for
//! as long as its velocity happens to be zero -- gravity puts it back in motion
//! on the very next tick and it sinks through whatever it landed on.
//!
//! This drives a real `HyperionCore` world through `world.progress()`, so what
//! is under test is the shipped system and not a copy of its arithmetic. The
//! world has no terrain (`HyperionCore` installs `Blocks::empty`), which is why
//! the *entry* into the ground is checked against a real client in
//! `bedwars-bow-e2e` rather than here. What a blockless world can still say is
//! everything that follows from the state, and that is the half gravity was
//! quietly undoing.
//!
//! One test and not four, because `HyperionCore` builds rayon's global thread
//! pool and a second boot in the same process fails on it. Each phase below
//! uses its own entity, so they do not interact.

use flecs_ecs::core::{EntityView, EntityViewGet, World};
use glam::Vec3;
use hyperion::{
    HyperionCore,
    simulation::{
        Owner, Pitch, Position, Velocity, Yaw,
        entity_kind::EntityKind,
        metadata::arrow::InGround,
        projectile_motion::{SHAKE_TICKS, ShakeTime},
    },
};

/// An arrow flying flat at one block a tick, owned by a shooter the ray cast
/// will exclude.
fn arrow(world: &World) -> EntityView<'_> {
    let owner = world.entity();
    let entity = world.entity();
    entity
        .add_enum(EntityKind::Arrow)
        .set(Position::new(0.0, 100.0, 0.0))
        .set(Velocity::new(1.0, 0.0, 0.0))
        .set(Yaw::new(0.0))
        .set(Pitch::new(0.0))
        .set(Owner::new(*owner));
    entity
}

fn state(entity: EntityView<'_>) -> (Vec3, Vec3) {
    entity.get::<(&Position, &Velocity)>(|(position, velocity)| (**position, velocity.0))
}

fn tick(world: &World, times: u32) {
    for _ in 0..times {
        world.progress();
    }
}

#[test]
fn a_landed_arrow_holds_still_and_a_flying_one_does_not() {
    let world = World::new();
    world.import::<HyperionCore>();

    // An arrow is told about `IN_GROUND` at all. The prefab carrying the
    // tracked field has to be applied by kind, or the flag is a server-side
    // boolean no client ever hears about and the client keeps flying an arrow
    // the server has stopped.
    let embedded = arrow(&world);
    assert_eq!(
        embedded.try_get::<&InGround>(|flag| **flag),
        Some(false),
        "every arrow should inherit the IN_GROUND tracked field, defaulted to false"
    );
    // And the shake clock, seeded by `seed_projectile_motion` for every kind
    // whose tick is `AbstractArrow.tick`. Without it the impact has nowhere to
    // write the seven ticks and the countdown below has nothing to count.
    assert_eq!(
        embedded.try_get::<&ShakeTime>(|shake| *shake),
        Some(ShakeTime(0)),
        "every arrow should be seeded with a shake clock, at rest"
    );

    // What `onHitBlock` leaves behind, except for the velocity: flagged, and
    // shaking. Set by hand because a blockless world has nothing to hit.
    //
    // The velocity is left at a block a tick on purpose, and that is the whole
    // point of this phase. `update_projectile_positions` skips a projectile
    // that is not moving anyway, so an embedded arrow whose velocity is also
    // zero cannot tell "in the ground" from "not moving" -- it holds still
    // either way, and a test built on it would pass with the in-ground branch
    // deleted. Vanilla's rule is the stronger one: `AbstractArrow.tick` returns
    // before it looks at the movement at all (lines 184-200), so an arrow in
    // the ground stays put whatever its velocity says. A game module that
    // knocks a stuck arrow loose has to clear the flag, not just write a
    // velocity.
    embedded.set(InGround::new(true));
    embedded.set(ShakeTime(SHAKE_TICKS));
    let (resting, _) = state(embedded);

    // One tick short of the full count, so the value is still positive: a
    // countdown that jumped straight to zero would pass an "eventually zero"
    // assertion just as loudly.
    tick(&world, u32::from(SHAKE_TICKS) - 1);
    assert_eq!(
        embedded.get::<&ShakeTime>(|shake| *shake),
        ShakeTime(1),
        "the shake should count down one tick at a time"
    );

    tick(&world, 20);
    assert_eq!(
        embedded.get::<&ShakeTime>(|shake| *shake),
        ShakeTime(0),
        "the shake should stop at zero rather than wrap"
    );

    let (position, velocity) = state(embedded);
    assert_eq!(
        position, resting,
        "an arrow in the ground should not move, whatever its velocity says; it drifted to \
         {position}"
    );
    assert_eq!(
        velocity,
        Vec3::new(1.0, 0.0, 0.0),
        "an arrow in the ground should lose nothing to drag and gain nothing from gravity; its \
         velocity became {velocity}"
    );

    // The guard for the guard: the same twenty ticks, without the flag, must
    // move the arrow. Otherwise everything above passes on a world that never
    // ticked at all.
    let flying = arrow(&world);
    let (start, _) = state(flying);
    tick(&world, 20);

    let (position, velocity) = state(flying);
    assert!(
        position.x > start.x + 1.0,
        "a flying arrow should have travelled; it is at {position}"
    );
    assert!(
        velocity.y < -0.5,
        "a flying arrow should be falling by now; its velocity is {velocity}"
    );
}
