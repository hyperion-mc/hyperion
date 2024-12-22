#![feature(assert_matches)]
#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use std::{assert_matches::assert_matches, collections::HashSet};

use approx::assert_relative_eq;
use flecs_ecs::{core::{
    flecs, Entity, EntityView, EntityViewGet, QueryBuilderImpl, SystemAPI, World, WorldGet
}, macros::system};
use geometry::{aabb::Aabb, ray::Ray};
use glam::{IVec3, Vec3};
use hyperion::{
    runtime::AsyncRuntime, simulation::{
        blocks::Blocks, entity_kind::EntityKind, event, EntitySize, Owner, Pitch, Position, Spawn, Velocity, Yaw
    }, spatial::{self, Spatial, SpatialIndex, SpatialModule}, storage::EventQueue, HyperionCore
};
use spatial::get_first_collision;

#[test]
fn test_get_first_collision() {
    let world = World::new();
    world.import::<HyperionCore>();
    world.import::<SpatialModule>();
    world.import::<hyperion_utils::HyperionUtilsModule>();
    world.get::<&AsyncRuntime>(|runtime| {
        const URL: &str = "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";

        let f = hyperion_utils::cached_save(&world, URL);

        let save = runtime.block_on(f).unwrap_or_else(|e| {
            panic!("failed to download map {URL}: {e}");
        });

        world.set(Blocks::new(&world, &save).unwrap());
    });

    // Make all entities have Spatial component so they are spatially indexed
    world
        .observer::<flecs::OnAdd, ()>()
        .with_enum_wildcard::<EntityKind>()
        .each_entity(|entity, ()| {
            entity.add::<Spatial>();
        });

    // Create a player entity
    let player = world
        .entity_named("test_player")
        .add_enum(EntityKind::Player)
        .set(EntitySize::default())
        .set(Position::new(0.0, 21.0, 0.0))
        .set(Yaw::new(0.0))
        .set(Pitch::new(90.0));

    // Function to spawn arrows at different angles
    fn spawn_arrow(world: &World, position: Vec3, direction: Vec3) -> EntityView<'_> {
        world
            .entity()
            .add_enum(EntityKind::Arrow)
            .set(Velocity::new(direction.x, direction.y, direction.z))
            .set(Position::new(position.x, position.y, position.z))
    }

    // Spawn arrows at different angles
    let arrow_velocities = [
        Vec3::new(0.0, -1.0, 0.0),
        // Vec3::new(1.0, 0.0, 0.0),
    ];

    let arrows: Vec<EntityView<'_>> = arrow_velocities
        .iter()
        .map(|velocity| {
            spawn_arrow(&world, Vec3::new(0.0, 21.0, 0.0), *velocity).set(Owner::new(*player))
        })
        .collect();

    // Progress the world to ensure that the index is updated
    world.progress();

    // Get all entities with Position and Velocity components
    arrows.iter().for_each(|arrow| {
        arrow.get::<(&Position, &Velocity)>(|(position, velocity)| {
            println!("position: {position:?}");
            println!("velocity: {velocity:?}");
        });
    });

    world.progress();

    // Get all entities with Position and Velocity components
    arrows.iter().for_each(|arrow| {
        arrow.get::<(&Position, &Velocity)>(|(position, velocity)| {
            println!("position: {position:?}");
            println!("velocity: {velocity:?}");
        });
    });
}
