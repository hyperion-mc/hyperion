#![feature(assert_matches)]
#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use std::{assert_matches::assert_matches, collections::HashSet};

use approx::assert_relative_eq;
use flecs_ecs::core::{QueryBuilderImpl, SystemAPI, World, WorldGet, flecs};
use geometry::{aabb::Aabb, ray::Ray};
use glam::Vec3;
use hyperion::{
    HyperionCore,
    simulation::{EntitySize, Position, entity_kind::EntityKind},
    spatial,
};
use spatial::{Spatial, SpatialIndex, SpatialModule};

#[test]
fn spatial() {
    let world = World::new();
    world.import::<HyperionCore>();
    world.import::<SpatialModule>();

    // Make all entities have Spatial component so they are spatially indexed
    world
        .observer::<flecs::OnAdd, ()>()
        .with_enum_wildcard::<EntityKind>()
        .each_entity(|entity, ()| {
            entity.add::<Spatial>();
        });

    let zombie = world
        .entity_named("test_zombie")
        .add_enum(EntityKind::Zombie)
        .set(EntitySize::default())
        .set(Position::new(0.0, 0.0, 0.0));

    let player = world
        .entity_named("test_player")
        .add_enum(EntityKind::Player)
        .set(EntitySize::default())
        .set(Position::new(10.0, 0.0, 0.0));

    // progress one tick to ensure that the index is updated
    world.progress();

    world.get::<&SpatialIndex>(|spatial| {
        let closest = spatial
            .closest_to(Vec3::new(1.0, 2.0, 0.0), &world)
            .expect("there to be a closest entity");
        assert_eq!(closest, zombie);

        let closest = spatial
            .closest_to(Vec3::new(11.0, 2.0, 0.0), &world)
            .expect("there to be a closest entity");
        assert_eq!(closest, player);

        let big_aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 100.0, 100.0));

        let collisions: HashSet<_> = spatial.get_collisions(big_aabb, &world).collect();
        assert!(
            collisions.contains(&zombie),
            "zombie should be in collisions"
        );
        assert!(
            collisions.contains(&player),
            "player should be in collisions"
        );

        let ray = Ray::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let (first, distance) = spatial.first_ray_collision(ray, &world).unwrap();
        assert_eq!(first, zombie);
        assert_relative_eq!(distance.into_inner(), 0.0);

        let ray = Ray::from_points(Vec3::new(12.0, 0.0, 0.0), Vec3::new(13.0, 1.0, 1.0));
        assert_matches!(spatial.first_ray_collision(ray, &world), None);
    });
}
