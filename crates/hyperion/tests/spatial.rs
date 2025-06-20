#![feature(assert_matches)]
#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use std::{assert_matches::assert_matches, collections::HashSet};

use approx::assert_relative_eq;
use bevy::{app::FixedMain, prelude::*};
use geometry::{aabb::Aabb, ray::Ray};
use glam::Vec3;
use hyperion::{
    HyperionCore,
    simulation::{EntitySize, Position, entity_kind::EntityKind},
    spatial,
};
use spatial::{Spatial, SpatialIndex, SpatialPlugin};

#[test]
fn spatial() {
    let mut app = App::new();
    app.add_plugins(SpatialPlugin);

    let zombie = app
        .world_mut()
        .spawn((EntitySize::default(), Position::new(0.0, 0.0, 0.0), Spatial))
        .id();

    let player = app
        .world_mut()
        .spawn((
            EntitySize::default(),
            Position::new(10.0, 0.0, 0.0),
            Spatial,
        ))
        .id();

    // progress one tick to ensure that the index is updated
    FixedMain::run_fixed_main(app.world_mut());

    let system = app.register_system(
        move |spatial: Res<'_, SpatialIndex>, query: Query<'_, '_, (&Position, &EntitySize)>| {
            let closest = spatial
                .closest_to(Vec3::new(1.0, 2.0, 0.0), query)
                .expect("there to be a closest entity");
            assert_eq!(closest, zombie);

            let closest = spatial
                .closest_to(Vec3::new(11.0, 2.0, 0.0), query)
                .expect("there to be a closest entity");
            assert_eq!(closest, player);

            let big_aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 100.0, 100.0));

            let collisions: HashSet<_> = spatial.get_collisions(big_aabb, query).collect();
            assert!(
                collisions.contains(&zombie),
                "zombie should be in collisions"
            );
            assert!(
                collisions.contains(&player),
                "player should be in collisions"
            );

            let ray = Ray::from_points(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
            let (first, distance) = spatial.first_ray_collision(ray, query).unwrap();
            assert_eq!(first, zombie);
            assert_relative_eq!(distance.into_inner(), 0.0);

            let ray = Ray::from_points(Vec3::new(12.0, 0.0, 0.0), Vec3::new(13.0, 1.0, 1.0));
            assert_matches!(spatial.first_ray_collision(ray, query), None);
        },
    );
    app.world_mut().run_system(system).unwrap();
}
