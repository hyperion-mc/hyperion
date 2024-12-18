#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use flecs_ecs::core::{EntityViewGet, World};
use hyperion::simulation::{Position, Uuid, Velocity, blocks::Blocks, entity_kind::EntityKind};

#[test]
fn arrow() {
    let world = World::new();
    world.import::<hyperion::HyperionCore>();

    // todo: make GenMapModule work without this
    world.import::<hyperion_utils::HyperionUtilsModule>();

    world.set(hyperion_utils::AppId {
        qualifier: "com".to_string(),
        organization: "andrewgazelka".to_string(),
        application: "hyperion-poc".to_string(),
    });

    let arrow = world.entity().add_enum(EntityKind::Arrow);

    assert!(
        arrow.has::<Uuid>(),
        "All entities should automatically be given a UUID."
    );

    arrow.get::<&Uuid>(|uuid| {
        assert_ne!(uuid.0, uuid::Uuid::nil(), "The UUID should not be nil.");
    });

    arrow
        .set(Velocity::new(0.0, 1.0, 0.0))
        .set(Position::new(0.0, 20.0, 0.0));

    println!("arrow = {arrow:?}\n");

    // The arrow is going to have physics; this currently only works if `BlockModule` is imported.
    assert!(!world.has::<Blocks>());
    world.import::<hyperion_genmap::GenMapModule>();
    assert!(world.has::<Blocks>());

    world.progress();

    arrow.get::<&Position>(|position| {
        // since velocity.y is 1.0, the arrow should be at y = 20.0 + 1.0
        assert_eq!(*position, Position::new(0.0, 21.0, 0.0));
    });

    world.progress();

    arrow.get::<&Position>(|position| {
        // since velocity.y is 1.0, the arrow should be at y = 20.0 + 1.0 + 1.0
        assert_eq!(*position, Position::new(0.0, 22.0, 0.0));
    });
}
