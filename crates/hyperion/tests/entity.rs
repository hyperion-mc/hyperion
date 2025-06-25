#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use bevy::{app::FixedMain, prelude::*};
use hyperion::{
    HyperionCore,
    simulation::{Owner, Position, Uuid, Velocity, entity_kind::EntityKind},
};
use serial_test::serial;

#[test]
#[serial]
#[ignore = "this test takes a SUPER long time to run due to https://github.com/hyperion-mc/hyperion/issues/909"]
fn arrow() {
    let mut app = App::new();
    app.add_plugins(HyperionCore);

    let world = app.world_mut();
    let owner = world.spawn_empty().id();
    let mut arrow = world.spawn(EntityKind::Arrow);
    let arrow_id = arrow.id();
    let arrow_uuid = arrow
        .get::<Uuid>()
        .expect("All entities should automatically be given a UUID");

    assert_ne!(
        arrow_uuid.0,
        uuid::Uuid::nil(),
        "The UUID should not be nil"
    );

    arrow.insert((
        Velocity::new(0.0, 1.0, 0.0),
        Position::new(0.0, 20.0, 0.0),
        Owner::new(owner),
    ));

    FixedMain::run_fixed_main(world);

    // since velocity.y is 1.0, the arrow should be at y = 20.0 + (1.0 * drag - gravity) = 20.947525
    assert_eq!(
        *world.entity(arrow_id).get::<Position>().unwrap(),
        Position::new(0.0, 20.947_525, 0.0)
    );

    FixedMain::run_fixed_main(world);

    // gravity! drag! this is what was returned from the test but I am unsure if it actually
    // what we should be getting
    // todo: make a bunch more tests and compare to the vanilla velocity and positions
    assert_eq!(
        *world.entity(arrow_id).get::<Position>().unwrap(),
        Position::new(0.0, 21.842_705, 0.0)
    );
}

#[test]
#[serial]
fn with_uuid() {
    let mut app = App::new();
    app.add_plugins(HyperionCore);

    let world = app.world_mut();
    let uuid = Uuid::new_v4();
    let arrow = world.spawn((uuid, EntityKind::Arrow));
    let arrow_uuid = *arrow.get::<Uuid>().unwrap();

    assert_eq!(
        arrow_uuid, uuid,
        "The entity UUID should not be overwritten with a randomly generated UUID"
    );
}
