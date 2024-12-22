use std::hint::black_box;

use geometry::{aabb::Aabb, ray::Ray};
use glam::Vec3;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use tango_bench::{
    DEFAULT_SETTINGS, IntoBenchmarks, MeasurementSettings, benchmark_fn, tango_benchmarks,
    tango_main,
};

// Helper function to generate random AABBs
fn random_aabb(rng: &mut SmallRng) -> Aabb {
    let x = rng.gen_range(-10.0..10.0);
    let y = rng.gen_range(-10.0..10.0);
    let z = rng.gen_range(-10.0..10.0);

    let width = rng.gen_range(0.1..5.0);
    let height = rng.gen_range(0.1..5.0);
    let depth = rng.gen_range(0.1..5.0);

    Aabb::new(
        Vec3::new(x, y, z),
        Vec3::new(x + width, y + height, z + depth),
    )
}

// Helper function to generate random rays
fn random_ray(rng: &mut SmallRng) -> Ray {
    let origin = Vec3::new(
        rng.gen_range(-15.0..15.0),
        rng.gen_range(-15.0..15.0),
        rng.gen_range(-15.0..15.0),
    );

    // Generate random direction and normalize
    let direction = Vec3::new(
        rng.gen_range(-1.0..1.0),
        rng.gen_range(-1.0..1.0),
        rng.gen_range(-1.0..1.0),
    )
    .normalize();

    Ray::new(origin, direction)
}

fn ray_intersection_benchmarks() -> impl IntoBenchmarks {
    let mut benchmarks = Vec::new();

    // Test different AABB sizes
    for &size in &[0.1, 1.0, 10.0] {
        benchmarks.push(benchmark_fn(
            format!("ray_intersection/aabb_size_{size}"),
            move |b| {
                let mut rng = SmallRng::seed_from_u64(b.seed);
                let aabb = Aabb::new(Vec3::new(-size, -size, -size), Vec3::new(size, size, size));
                b.iter(move || {
                    let ray = random_ray(&mut rng);
                    black_box(aabb.intersect_ray(&ray))
                })
            },
        ));
    }

    // Test rays from different positions
    for &distance in &[1.0, 5.0, 20.0] {
        benchmarks.push(benchmark_fn(
            format!("ray_intersection/ray_distance_{distance}"),
            move |b| {
                let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
                b.iter(move || {
                    let origin = Vec3::new(distance, 0.0, 0.0);
                    let ray = Ray::new(origin, -origin.normalize());
                    black_box(aabb.intersect_ray(&ray))
                })
            },
        ));
    }

    benchmarks
}

fn overlap_benchmarks() -> impl IntoBenchmarks {
    let mut benchmarks = Vec::new();

    // Test different overlap scenarios
    benchmarks.push(benchmark_fn("overlap/no_overlap", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb1 = Aabb::new(Vec3::ZERO, Vec3::ONE);
        b.iter(move || {
            let aabb2 = random_aabb(&mut rng).move_by(Vec3::new(2.0, 2.0, 2.0));
            black_box(Aabb::overlap(&aabb1, &aabb2))
        })
    }));

    benchmarks.push(benchmark_fn("overlap/partial_overlap", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb1 = Aabb::new(Vec3::ZERO, Vec3::ONE);
        b.iter(move || {
            let aabb2 = random_aabb(&mut rng).move_by(Vec3::new(0.5, 0.5, 0.5));
            black_box(Aabb::overlap(&aabb1, &aabb2))
        })
    }));

    benchmarks.push(benchmark_fn("overlap/full_containment", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb1 = Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0));
        b.iter(move || {
            let aabb2 = random_aabb(&mut rng);
            black_box(Aabb::overlap(&aabb1, &aabb2))
        })
    }));

    benchmarks
}

fn point_containment_benchmarks() -> impl IntoBenchmarks {
    let mut benchmarks = Vec::new();

    benchmarks.push(benchmark_fn("point_containment/inside", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        b.iter(move || {
            let point = Vec3::new(
                rng.gen_range(-0.9..0.9),
                rng.gen_range(-0.9..0.9),
                rng.gen_range(-0.9..0.9),
            );
            black_box(aabb.contains_point(point))
        })
    }));

    benchmarks.push(benchmark_fn("point_containment/outside", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        b.iter(move || {
            let point = Vec3::new(
                rng.gen_range(1.1..2.0),
                rng.gen_range(1.1..2.0),
                rng.gen_range(1.1..2.0),
            );
            black_box(aabb.contains_point(point))
        })
    }));

    benchmarks.push(benchmark_fn("point_containment/boundary", move |b| {
        let mut rng = SmallRng::seed_from_u64(b.seed);
        let aabb = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        b.iter(move || {
            // Generate points very close to the boundary
            let point = Vec3::new(
                rng.gen_range(-1.001..1.001),
                rng.gen_range(-1.001..1.001),
                rng.gen_range(-1.001..1.001),
            );
            black_box(aabb.contains_point(point))
        })
    }));

    benchmarks
}

// Custom settings for more stable results
const SETTINGS: MeasurementSettings = MeasurementSettings {
    min_iterations_per_sample: 1000,
    cache_firewall: Some(64), // 64KB cache firewall
    yield_before_sample: true,
    randomize_stack: Some(4096), // 4KB stack randomization
    ..DEFAULT_SETTINGS
};

tango_benchmarks!(
    ray_intersection_benchmarks(),
    overlap_benchmarks(),
    point_containment_benchmarks()
);
tango_main!(SETTINGS);
