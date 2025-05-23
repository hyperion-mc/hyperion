//! The goal of this is to test whether atomics of thread locals make more sense

use std::hint::black_box;

use divan::Bencher;
use flecs_ecs::prelude::World;
use hyperion::storage::raw::RawQueue;

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
