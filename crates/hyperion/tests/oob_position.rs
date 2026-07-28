//! A single client-supplied movement packet must not be able to crash the
//! server. ENG-10914: `MovePlayerPos { x: 2_000_000.0, .. }` sits far inside
//! the Minecraft world border yet its chunk key `2_000_000 >> 4 == 125_000`
//! overflows the `i16` this server keys chunks by. That overflow reached the
//! chunk math and took the whole process down for every connected player:
//! a `debug_assert` in `Blocks::get_blocks` in a debug build, a narrowing
//! `unwrap` / silent `as_i16vec2` wrap in release.
//!
//! The load-bearing fix is a bound in the move handler, exercised end-to-end
//! by the `oob-move-e2e` wire gate (a client sends this exact packet and the
//! server keeps ticking). These unit checks pin the defence-in-depth layer:
//! the coordinate conversions themselves are now total, so they cannot crash
//! whatever calls them.

use glam::IVec3;
use hyperion::{
    HyperionCore,
    simulation::{Position, blocks::Blocks},
};

/// The horizontal block coordinate from the ENG-10914 crashing packet.
const CRASH_BLOCK: i32 = 2_000_000;

#[test]
fn to_chunk_does_not_panic_on_the_crashing_coordinate() {
    // Runs every tick for every player; used to `try_from().unwrap()` panic.
    let _ = Position::new(CRASH_BLOCK as f32, 65.0, 4.0).to_chunk();
    // And the extremes, for good measure.
    let _ = Position::new(f32::MAX, 65.0, f32::MIN).to_chunk();
    let _ = Position::new(f32::INFINITY, 65.0, f32::NAN).to_chunk();
}

#[test]
fn get_blocks_does_not_panic_on_the_crashing_coordinate() {
    let world = flecs_ecs::core::World::new();
    world.import::<HyperionCore>();

    let blocks = Blocks::empty(&world);

    // The block range a player's bounding box at the crashing position would
    // ask about. Before the fix this panicked (debug) or walked a wrapped
    // chunk range (release); now it must simply return, touching nothing,
    // because no chunk near there is loaded.
    let start = IVec3::new(CRASH_BLOCK, 64, 4);
    let end = IVec3::new(CRASH_BLOCK + 1, 66, 5);

    let mut visited = 0_usize;
    let outcome: Option<()> = blocks.get_blocks(start, end, |_, _| {
        visited += 1;
        Some(())
    });
    assert_eq!(
        outcome,
        Some(()),
        "the walk ran to completion without panicking"
    );
    assert_eq!(visited, 0, "no chunk is loaded near the out-of-range query");
}
