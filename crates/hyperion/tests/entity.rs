#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use flecs_ecs::{
    core::{EntityViewGet, World, id},
    macros::Component,
    prelude::Module,
};
use hyperion::{
    simulation::{Owner, Position, Uuid, Velocity, entity_kind::EntityKind},
    spatial::SpatialModule,
};
use serial_test::serial;

#[derive(Component)]
struct TestModule;

impl Module for TestModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world.import::<SpatialModule>();
    }
}

#[test]
#[serial]
fn arrow() {
    let world = World::new();
    world.import::<TestModule>();

    let arrow = world.entity().add_enum(EntityKind::Arrow);
    let owner = world.entity();

    assert!(
        arrow.has(id::<Uuid>()),
        "All entities should automatically be given a UUID."
    );

    arrow.get::<&Uuid>(|uuid| {
        assert_ne!(uuid.0, uuid::Uuid::nil(), "The UUID should not be nil.");
    });

    arrow
        .set(Velocity::new(0.0, 1.0, 0.0))
        .set(Position::new(0.0, 20.0, 0.0))
        .set(Owner::new(*owner));

    world.progress();

    // What the numbers should be is not decided here. This file used to assert
    // two positions that were, in its own words, "what was returned from the
    // test but I am unsure if it actually what we should be getting", with a
    // note asking for a comparison against vanilla. `tests/differential.rs` is
    // that comparison: it replays a recording of the real server tick by tick.
    // All that is left here is the part vanilla has no opinion about, which is
    // that an arrow moves at all rather than sitting where it was put.
    arrow.get::<&Position>(|position| {
        assert!(
            position.y > 20.0,
            "an arrow launched upwards should have moved: {position:?}"
        );
    });
}
