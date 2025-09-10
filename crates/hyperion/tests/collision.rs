#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use bevy::{app::FixedMain, prelude::*};
use glam::Vec3;
use hyperion::{
    HyperionCore,
    simulation::{EntitySize, Owner, Pitch, Position, Velocity, Yaw, entity_kind::EntityKind},
    spatial::Spatial,
};

#[test]
fn test_get_first_collision() {
    /// Function to spawn arrows at different angles
    fn spawn_arrow(world: &mut World, position: Vec3, direction: Vec3, owner: Owner) -> Entity {
        tracing::debug!("Spawning arrow at position: {position:?} with direction: {direction:?}");
        world
            .spawn((
                EntityKind::Arrow,
                Spatial,
                Velocity::new(direction.x, direction.y, direction.z),
                Position::new(position.x, position.y, position.z),
                owner,
            ))
            .id()
    }

    let mut app = App::new();

    app.add_plugins((HyperionCore, hyperion_genmap::GenMapPlugin));

    let world = app.world_mut();

    // Create a player entity
    let player = world
        .spawn((
            EntityKind::Player,
            EntitySize::default(),
            Position::new(0.0, 21.0, 0.0),
            Yaw::new(0.0),
            Pitch::new(90.0),
        ))
        .id();

    // Spawn arrows at different angles
    let arrow_velocities = [Vec3::new(0.0, -1.0, 0.0)];

    let arrows: Vec<Entity> = arrow_velocities
        .iter()
        .map(|velocity| {
            spawn_arrow(
                world,
                Vec3::new(0.0, 21.0, 0.0),
                *velocity,
                Owner::new(player),
            )
        })
        .collect();

    // Progress the world to ensure that the index is updated
    FixedMain::run_fixed_main(world);

    let mut query = world.query::<(&Position, &Velocity)>();

    // Get all entities with Position and Velocity components
    for arrow in &arrows {
        let (position, velocity) = query.get(world, *arrow).unwrap();
        println!("position: {position:?}");
        println!("velocity: {velocity:?}");
    }

    FixedMain::run_fixed_main(world);

    // Get all entities with Position and Velocity components
    for arrow in &arrows {
        let (position, velocity) = query.get(world, *arrow).unwrap();
        println!("position: {position:?}");
        println!("velocity: {velocity:?}");
    }
}
