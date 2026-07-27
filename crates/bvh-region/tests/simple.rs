use std::collections::HashSet;

use approx::assert_relative_eq;
use bvh_region::Bvh;
use geometry::{
    aabb::{Aabb, OrderedAabb},
    ray::Ray,
};
use glam::Vec3;
use ordered_float::NotNan;
use proptest::prelude::*;

const fn copied<T: Copy>(value: &T) -> T {
    *value
}

// Helper function to compute the squared distance from a point to an Aabb.
// This logic might differ depending on how you define Aabb.
fn aabb_distance_squared(aabb: &Aabb, p: Vec3) -> f64 {
    let p = p.as_dvec3();
    let min = aabb.min.as_dvec3(); // Ensure you have aabb.min / aabb.max as Vec3
    let max = aabb.max.as_dvec3();
    let clamped = p.clamp(min, max);
    p.distance_squared(clamped)
}

#[test]
fn test_bvh_vs_brute_known_case() {
    let elements = vec![
        Aabb::new((0.0, 0.0, 0.0), (10.0, 10.0, 10.0)),
        Aabb::new((5.0, 5.0, 0.0), (15.0, 15.0, 10.0)),
    ];
    let ray = Ray::new(Vec3::new(2.0, 2.0, 2.0), Vec3::new(1.0, 0.0, 0.0));

    // Brute force:
    let brute_closest = brute_force_closest_ray(&elements, ray);

    // BVH:
    let bvh = Bvh::build(elements.clone(), |x| *x);
    let bvh_closest = bvh.first_ray_collision(ray, |x| *x);

    assert_eq!(brute_closest.map(|x| x.1), bvh_closest.map(|x| x.1));
}

#[test]
fn test_multiple_aabbs_along_ray() {
    let elements = vec![
        Aabb::new(Vec3::new(1.0, -0.5, -0.5), Vec3::new(2.0, 0.5, 0.5)),
        Aabb::new(Vec3::new(3.0, -1.0, -1.0), Vec3::new(4.0, 1.0, 1.0)),
        Aabb::new(Vec3::new(5.0, -0.5, -0.5), Vec3::new(6.0, 0.5, 0.5)),
    ];

    let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
    let bvh = Bvh::build(elements.clone(), |x| *x);
    let (closest, dist) = bvh
        .first_ray_collision(ray, |x| *x)
        .expect("Should find intersection");
    assert_eq!(
        closest, &elements[0],
        "Closest AABB should be the first one"
    );
    assert!(dist < NotNan::new(2.0).unwrap());
}

proptest! {
    #[test]
    fn test_rays_origin_inside_aabb(
        // Smaller ranges might reduce flakiness
        elements in prop::collection::vec(
            (0.0..10.0f32, 0.0..10.0f32, 0.0..10.0f32, 10.0..20.0f32, 10.0..20.0f32, 10.0..20.0f32)
                .prop_map(|tuple| {
                    // Guaranteed that min < max for all coords
                    Aabb::from(tuple)
                }),
            1..20
        ),
        origin in (5.0..15.0f32, 5.0..15.0f32, 5.0..15.0f32), // Potentially inside some AABBs
        direction in (-10.0..10.0f32, -10.0..10.0f32, -10.0..10.0f32)
    ) {
        // Avoid zero direction:
        prop_assume!(direction != (0.0, 0.0, 0.0));
        let ray = Ray::new(Vec3::new(origin.0, origin.1, origin.2), Vec3::new(direction.0, direction.1, direction.2));

        let bvh = Bvh::build(elements.clone(), |x| *x);
        let bvh_closest = bvh.first_ray_collision(ray, |x| *x);
        let brute_closest = brute_force_closest_ray(&elements, ray);

        match (bvh_closest, brute_closest) {
            (None, None) => {}
            (Some((bvh_aabb, bvh_t)), Some((brute_aabb, brute_t))) => {
                let diff = (bvh_t.into_inner() - brute_t.into_inner()).abs();
                prop_assert!(diff < 1e-6, "Rays from inside differ in intersection distance ... {bvh_aabb:?}, {brute_aabb:?}");
            },
            _ => {
                // If one returns None and the other doesn't, it might indicate special case handling
                prop_assert!(false, "Mismatch in inside-ray intersection");
            }
        }
    }
}

#[test]
fn simple() {
    let elements = vec![Aabb {
        min: Vec3::new(-1.470_215_5e30, 0.0, 0.0),
        max: Vec3::new(0.0, 0.0, 0.0),
    }];

    let target = Vec3::new(0.0, 0.0, 0.0);
    let bvh = Bvh::build(elements.clone(), copied);
    let (closest, dist2) = bvh.get_closest(target, copied).unwrap();

    assert_eq!(closest, &elements[0]);
    assert_relative_eq!(dist2, 0.0);
}

#[test]
fn test_empty_get_closest() {
    let elements = vec![];

    let target = Vec3::new(0.0, 0.0, 0.0);
    let bvh = Bvh::build(elements, copied);

    assert_eq!(bvh.get_closest(target, copied), None);
}

proptest! {
    #[test]
    fn test_get_closest_correctness(
        elements in prop::collection::vec(
            // Generate random AABBs by picking two random points and making one the min and the other the max.
            (any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>())
                .prop_map(|(x1, y1, z1, x2, y2, z2)| {
                    let min_x = x1.min(x2);
                    let max_x = x1.max(x2);
                    let min_y = y1.min(y2);
                    let max_y = y1.max(y2);
                    let min_z = z1.min(z2);
                    let max_z = z1.max(z2);
                    Aabb::from([min_x, min_y, min_z, max_x, max_y, max_z])
                }),
            1..50 // vary the number of elements from small to moderate
        ),
        target in (any::<f32>(), any::<f32>(), any::<f32>())
    ) {
        let target_vec = Vec3::new(target.0, target.1, target.2);
        let bvh = Bvh::build(elements.clone(), copied);

        let closest_bvh = bvh.get_closest(target_vec, copied);

        // Compute the closest element by brute force
        let mut best: Option<(&Aabb, f64)> = None;
        for aabb in &elements {
            let dist = aabb_distance_squared(aabb, target_vec);
            if let Some((_, best_dist)) = best {
                if dist < best_dist {
                    best = Some((aabb, dist));
                }
            } else {
                best = Some((aabb, dist));
            }
        }

        // Compare results
        match (closest_bvh, best) {
            (Some((bvh_aabb, bvh_dist)), Some((brute_aabb, brute_dist))) => {
                if bvh_dist.is_infinite() && brute_dist.is_infinite() {
                    // If both are infinite, they should return the same element
                    prop_assert_eq!(bvh_aabb, brute_aabb);
                } else {
                    // Check that the distances are essentially the same
                    prop_assert!((bvh_dist - brute_dist).abs() < 1e-6, "Distances differ significantly; BVH: {bvh_dist}, brute force: {brute_dist}");

                    let target = Vec3::new(target.0, target.1, target.2);
                    let calculated_bvh_dist = bvh_aabb.dist2(target);
                    let calculated_brute_dist = brute_aabb.dist2(target);
                    prop_assert!((calculated_bvh_dist - calculated_brute_dist).abs() < 1e-6, "Distances differ significantly; BVH: {calculated_bvh_dist}, brute force: {calculated_brute_dist}");

                    // We are commenting this out because there might be some cases
                    // where there are multiple "correct" answers.
                    // prop_assert_eq!(bvh_aabb, brute_aabb);
                }

            },
            (None, None) => {
                // If there are no elements, both should return None
                prop_assert!(true);
            },
            (x,y) => {
                // If one returns None and the other doesn't, there's a mismatch
                prop_assert!(false, "Mismatch between BVH closest and brute force closest; BVH: {x:?}, brute force: {y:?}");
            }
        }
    }
}

proptest! {
    #[test]
    fn test_range_correctness(
        elements in prop::collection::vec(
            (any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>())
                .prop_map(|(x1, y1, z1, x2, y2, z2)| {
                    let min_x = x1.min(x2);
                    let max_x = x1.max(x2);
                    let min_y = y1.min(y2);
                    let max_y = y1.max(y2);
                    let min_z = z1.min(z2);
                    let max_z = z1.max(z2);
                    Aabb::from([min_x, min_y, min_z, max_x, max_y, max_z])
                }),
            1..50
        ),
        target in (any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>())
            .prop_map(|(x1, y1, z1, x2, y2, z2)| {
                let min_x = x1.min(x2);
                let max_x = x1.max(x2);
                let min_y = y1.min(y2);
                let max_y = y1.max(y2);
                let min_z = z1.min(z2);
                let max_z = z1.max(z2);
                Aabb::from([min_x, min_y, min_z, max_x, max_y, max_z])
            })
    ) {
        let bvh = Bvh::build(elements.clone(), copied);

        // Compute brute force collisions
        let mut brute_force_set = HashSet::new();
        for aabb in &elements {
            if aabb.collides(&target) {
                let aabb = OrderedAabb::try_from(*aabb).unwrap();
                brute_force_set.insert(aabb);
            }
        }

        // Compute BVH collisions
        let mut bvh_set = HashSet::new();

        for candidate in bvh.range(target, copied) {
            // Find index of candidate in `elements`:
            let candidate = OrderedAabb::try_from(*candidate).unwrap();
            bvh_set.insert(candidate);
        }

        // Compare sets
        prop_assert_eq!(&bvh_set, &brute_force_set, "Mismatch between BVH range and brute force collision sets: {:?} != {:?}", bvh_set, brute_force_set);
    }
}

/// Computes the closest intersection of `ray` with the list of `elements` via brute force.
fn brute_force_closest_ray(elements: &[Aabb], ray: Ray) -> Option<(&Aabb, NotNan<f32>)> {
    let mut closest_t = NotNan::new(f32::INFINITY).unwrap();
    let mut closest_elem = None;

    for aabb in elements {
        if let Some(t) = aabb.intersect_ray(&ray)
            && t < closest_t
            && t.into_inner() >= 0.0
        {
            closest_t = t;
            closest_elem = Some(aabb);
        }
    }

    closest_elem.map(|e| (e, closest_t))
}

proptest! {
    #[test]
    fn test_get_closest_ray_correctness(
        elements in prop::collection::vec(
            // Generate random AABBs by picking two random points and making one the min and the other the max.
            (-1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32)
                .prop_map(|(x1, y1, z1, x2, y2, z2)| {
                    let min_x = x1.min(x2);
                    let max_x = x1.max(x2);
                    let min_y = y1.min(y2);
                    let max_y = y1.max(y2);
                    let min_z = z1.min(z2);
                    let max_z = z1.max(z2);
                    Aabb::from([min_x, min_y, min_z, max_x, max_y, max_z])
                }),
            1..50 // vary the number of elements
        ),
        origin in (-1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32),
        direction in (-1000.0..1000.0f32, -1000.0..1000.0f32, -1000.0..1000.0f32)
    ) {
        // If the direction is zero, we skip this test case. A ray with zero direction doesn't make sense.
        let dir_vec = Vec3::new(direction.0, direction.1, direction.2);
        let zero_vec = Vec3::new(0.0, 0.0, 0.0);

        if dir_vec == zero_vec {
            return Ok(());
        }

        let ray = Ray::new(
            Vec3::new(origin.0, origin.1, origin.2),
            dir_vec
        );

        let bvh = Bvh::build(elements.clone(), copied);
        let bvh_closest = bvh.first_ray_collision(ray, copied);
        let brute_closest = brute_force_closest_ray(&elements, ray);

        match (bvh_closest, brute_closest) {
            (None, None) => {
                // Both found no intersections.
            },
            (Some((bvh_aabb, bvh_t)), Some((brute_aabb, brute_t))) => {
                // Check that the chosen elements and intersection distances are close.
                // Because multiple AABBs might have the exact same intersection distance, we can't always assert equality of the element references.
                // But at minimum, we check that the distances are very close.
                let diff = (bvh_t.into_inner() - brute_t.into_inner()).abs();
                prop_assert!(diff < 1e-6, "Distances differ significantly; BVH: bvh_t: {bvh_t}, brute force: {brute_t}, aabb1: {bvh_aabb:?}, aabb2: {brute_aabb:?}");

                // If desired (and if you trust that intersections are unique), you could also check:
                // prop_assert_eq!(bvh_elem, brute_elem);
            },
            (bvh_val, brute_val) => {
                // Mismatch: One found an intersection, the other didn't.
                prop_assert!(false, "Mismatch between BVH closest ray and brute force closest ray; BVH: {:?}, brute force: {:?}", bvh_val, brute_val);
            }
        }
    }
}
