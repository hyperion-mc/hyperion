//! The goal of this is to test whether atomics of thread locals make more sense

use std::hint::black_box;

use divan::Bencher;
use flecs_ecs::prelude::World;
use hyperion::{
    storage::{ThreadHeaplessVec, ThreadLocalSoaVec, ThreadLocalVec, raw::RawQueue},
    util::SendableRef,
};

const THREADS: &[usize] = &[1, 2, 4, 8];

fn main() {
    divan::main();
}

const COUNT: usize = 16_384;

#[divan::bench(
    args = THREADS,
)]
fn populate_queue(bencher: Bencher<'_, '_>, threads: usize) {
    let world = World::new();
    world.set_stage_count(4);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();

    bencher
        .with_inputs(|| RawQueue::new(COUNT * 4))
        .bench_local_values(|elems| {
            pool.broadcast(|_| {
                for _ in 0..COUNT {
                    elems.push(42).unwrap();
                }
            });
        });
}

#[divan::bench(
    args = THREADS,
)]
fn populate_thread_local(bencher: Bencher<'_, '_>, threads: usize) {
    let world = World::new();
    world.set_stage_count(8);

    let stages: Vec<_> = (0..8).map(|i| world.stage(i)).map(SendableRef).collect();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();

    bencher
        .with_inputs(|| ThreadLocalVec::with_capacity(COUNT))
        .bench_local_values(|elems| {
            pool.broadcast(|ctx| {
                let index = ctx.index();
                let world = &stages[index];

                for _ in 0..COUNT {
                    elems.push(42, &world.0);
                }
            });
        });
}

#[divan::bench(
    args = THREADS,
)]
fn populate_thread_local_custom(bencher: Bencher<'_, '_>, threads: usize) {
    let world = World::new();
    world.set_stage_count(8);

    let stages: Vec<_> = (0..8).map(|i| world.stage(i)).map(SendableRef).collect();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();

    bencher
        .with_inputs(|| ThreadLocalSoaVec::with_capacity(COUNT))
        .bench_local_values(|elems| {
            pool.broadcast(|ctx| {
                let index = ctx.index();
                let world = &stages[index];

                for _ in 0..COUNT {
                    elems.push(42, &world.0);
                }
            });
        });
}

#[divan::bench]
fn is_empty_thread_local_heapless(bencher: Bencher<'_, '_>) {
    let mut elems = ThreadHeaplessVec::<i32, 32>::default();

    bencher.bench_local(|| black_box(elems.is_empty()));
}

#[divan::bench]
fn is_empty_thread_local(bencher: Bencher<'_, '_>) {
    let mut elems = ThreadLocalVec::<i32>::with_capacity(COUNT);

    bencher.bench_local(|| black_box(elems.is_empty()));
}

#[divan::bench]
fn is_empty_thread_local_custom(bencher: Bencher<'_, '_>) {
    let mut elems = ThreadLocalSoaVec::<i32>::with_capacity(COUNT);

    bencher.bench_local(|| black_box(elems.is_empty()));
}
