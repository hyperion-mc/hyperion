//! The collision shape table's own contract, independent of any consumer.
//!
//! `crates/hyperion/src/simulation/blocks/translate.rs` checks the named
//! geometry through the translation a 1.20.1 world needs. These check the
//! table as the crate publishes it: every id the block state table can produce
//! resolves, and nothing else does.

use hyperion_minecraft_proto::{
    block_state,
    collision_shape::{SHAPES, STATE_SHAPES, collision_shape},
};

#[test]
fn every_state_id_resolves_and_nothing_beyond_them_does() {
    for id in 0..block_state::STATE_COUNT {
        assert!(
            collision_shape(id).is_some(),
            "state {id} is inside the registry and has no shape"
        );
    }
    assert_eq!(collision_shape(block_state::STATE_COUNT), None);
    assert_eq!(collision_shape(u32::MAX), None);
}

#[test]
fn air_is_empty_and_stone_is_the_unit_cube() {
    // The two ends of the table, by name rather than by literal id, so this
    // still means what it says after a version bump renumbers everything.
    let air = block_state::state_id("minecraft:air", &[]).expect("26.2 has air");
    assert_eq!(collision_shape(air), Some(&[][..]));

    let stone = block_state::state_id("minecraft:stone", &[]).expect("26.2 has stone");
    assert_eq!(
        collision_shape(stone),
        Some(&[[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]][..])
    );
}

/// Blocks that did not exist in 1.20.1, which is what the old table could not
/// describe at all.
///
/// None of them can appear in a hyperion world today -- the world is read out
/// of 1.20.1 anvil regions -- so this is coverage rather than a fix. It is
/// here so that the day a 26.2 state does reach the collision path, the shape
/// is already right instead of missing.
#[test]
fn blocks_added_since_1_20_1_have_shapes() {
    for name in [
        "minecraft:crafter",
        "minecraft:trial_spawner",
        "minecraft:vault",
        "minecraft:pale_oak_log",
        "minecraft:copper_chest",
    ] {
        let id = block_state::default_state_id(name).unwrap_or_else(|| panic!("26.2 has {name}"));
        let shape = collision_shape(id).unwrap_or_else(|| panic!("{name} has no shape row"));
        assert!(
            !shape.is_empty(),
            "{name} is solid but has no collision box"
        );
    }
}

/// Every shape in the table is reachable, and every box is a real box.
///
/// A `min` above a `max` on any axis is an inside-out box: it intersects
/// nothing, so a block carrying one is one an entity falls through, and the
/// failure looks like a hole in the world rather than like bad data.
#[test]
fn the_table_is_well_formed() {
    let mut used = vec![false; SHAPES.len()];
    for &index in STATE_SHAPES {
        used[usize::from(index)] = true;
    }
    assert!(
        used.iter().all(|&used| used),
        "{} of {} shapes are referenced by no state",
        used.iter().filter(|used| !**used).count(),
        SHAPES.len()
    );

    for (index, shape) in SHAPES.iter().enumerate() {
        for box_ in *shape {
            let [min_x, min_y, min_z, max_x, max_y, max_z] = *box_;
            assert!(
                min_x < max_x && min_y < max_y && min_z < max_z,
                "shape {index} has an inside-out box: {box_:?}"
            );
        }
    }
}
