//! What a map file has to be true for, checked without a server.
//!
//! `terrain.rs` asserts most of this at boot, which means a bad map is a server
//! that panics on start rather than a test that fails. Boot is also the only
//! place it was ever checked, so a map could be broken for as long as nobody
//! started the game. These run in `cargo test`, need no world, no chunks and no
//! network, and fail on the map that is wrong rather than on the first one.

use std::collections::HashSet;

use smash::map::{ARENAS, Brush, HUB, MapSpec, parse};

/// Every shipped map, hub included, with a label for the failure message.
fn shipped() -> Vec<(String, MapSpec)> {
    let mut out = Vec::new();
    for (index, source) in core::iter::once(&HUB).chain(ARENAS).enumerate() {
        let spec =
            parse(source).unwrap_or_else(|error| panic!("map {index} does not parse: {error}"));
        out.push((format!("map {index} ({:?})", spec.name), spec));
    }
    out
}

/// Every block position a map's brushes cover.
///
/// The same rasterisation `terrain.rs` feeds to `set_block`, collected instead
/// of written, so a test can ask what a map is solid at without a world.
fn solid(spec: &MapSpec) -> HashSet<[i32; 3]> {
    let mut out = HashSet::new();
    for brush in &spec.brushes {
        brush.each_block(|at, _| {
            out.insert(at);
        });
    }
    out
}

#[test]
fn every_shipped_map_parses() {
    let maps = shipped();
    assert_eq!(
        maps.len(),
        ARENAS.len() + 1,
        "the hub and every arena should have parsed"
    );
    for (label, spec) in &maps {
        assert!(!spec.brushes.is_empty(), "{label} places no blocks at all");
    }
}

/// The three arenas the game asks a match to choose between.
#[test]
fn there_are_several_arenas_to_choose_between() {
    assert!(
        ARENAS.len() >= 3,
        "a rotation of fewer than three arenas is not a rotation"
    );
}

/// Two maps with one name would make selection by name ambiguous, and a
/// scoreboard that names the map would be lying on one of them.
#[test]
fn map_names_are_distinct() {
    let mut seen = HashSet::new();
    for (label, spec) in shipped() {
        assert!(
            seen.insert(spec.name),
            "{label} reuses the name {:?}",
            spec.name
        );
    }
}

/// The check `terrain.rs::stand_on_something` makes at boot, made here instead.
///
/// hyperion decides whether a player is on the ground by reading the block at
/// `ceil(y) - 1`, so a spawn one block too high leaves a player airborne from
/// the moment they arrive and every grounded ability silently refuses to fire.
#[test]
fn every_spawn_stands_on_geometry() {
    for (label, spec) in shipped() {
        let solid = solid(&spec);
        for spawn in &spec.spawns {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a spawn is within a few hundred blocks of its map origin"
            )]
            let below = [
                spawn.x.floor() as i32,
                spawn.y.ceil() as i32 - 1,
                spawn.z.floor() as i32,
            ];
            assert!(
                solid.contains(&below),
                "{label} spawns a player at {spawn} with nothing under them at {below:?}"
            );
        }
    }
}

/// A spawn at or below the kill plane kills whoever is put there, immediately.
/// `parse` refuses it; this is the shipped maps having a margin rather than
/// merely clearing it by a hair.
#[test]
fn every_spawn_clears_the_kill_plane() {
    for (label, spec) in shipped() {
        for spawn in &spec.spawns {
            assert!(
                spawn.y > spec.kill_y + 1.0,
                "{label} spawns at {spawn}, which is {:.1} above its kill plane {}",
                spawn.y - spec.kill_y,
                spec.kill_y
            );
        }
    }
}

/// The kill plane has to be under the map, not through it. A plane cutting the
/// geometry means a player standing on a real block dies for no visible reason.
#[test]
fn the_kill_plane_is_below_every_block() {
    for (label, spec) in shipped() {
        let lowest = solid(&spec)
            .iter()
            .map(|at| at[1])
            .min()
            .expect("a shipped map places blocks");
        assert!(
            (lowest as f32) > spec.kill_y,
            "{label} has a block at y {lowest}, at or below its kill plane {}",
            spec.kill_y
        );
    }
}

// --- the guards in `parse`, each watched to fire ---------------------------

fn rejected(source: &'static str) -> String {
    match parse(source) {
        Ok(spec) => panic!("expected a rejection, got the map {:?}", spec.name),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_map_with_no_name_is_refused() {
    assert!(rejected("kill_y 0\nspawn 0 1 0\nbox 0 0 0 0 0 0 minecraft:stone\n").contains("name"));
}

#[test]
fn a_map_with_no_spawn_is_refused() {
    let reason = rejected("name Nowhere\nkill_y 0\nbox 0 0 0 0 0 0 minecraft:stone\n");
    assert!(reason.contains("spawn"), "{reason}");
}

#[test]
fn a_map_with_no_kill_plane_is_refused() {
    let reason = rejected("name Endless\nspawn 0 1 0\n");
    assert!(reason.contains("kill_y"), "{reason}");
}

/// The bug the downloaded world shipped with: everyone died the instant they
/// were placed.
#[test]
fn a_kill_plane_above_the_spawns_is_refused() {
    let reason = rejected("name Fatal\nkill_y 70\nspawn 0 65 0\n");
    assert!(reason.contains("kill_y 70"), "{reason}");
    assert!(reason.contains("65"), "{reason}");
}

#[test]
fn an_unknown_directive_names_itself_and_its_line() {
    let reason = rejected("name Typo\nkill_y 0\nspawn 0 1 0\nsphre 0 0 0 1 minecraft:stone\n");
    assert!(reason.contains("line 4"), "{reason}");
    assert!(reason.contains("sphre"), "{reason}");
}

/// A bare `stone` is a block id in no namespace, and the block table would
/// reject it far from the file that wrote it.
#[test]
fn a_block_without_its_namespace_is_refused() {
    let reason = rejected("name Bare\nkill_y 0\nspawn 0 1 0\nbox 0 0 0 1 1 1 stone\n");
    assert!(reason.contains("minecraft:"), "{reason}");
}

#[test]
fn a_malformed_number_names_the_word_that_was_not_one() {
    let reason = rejected("name Fuzzy\nkill_y 0\nspawn 0 1 0\nsphere 0 0 0 big minecraft:stone\n");
    assert!(reason.contains("big"), "{reason}");
}

#[test]
fn trailing_junk_is_refused_rather_than_ignored() {
    let reason = rejected("name Extra\nkill_y 0\nspawn 0 1 0 sideways\n");
    assert!(reason.contains("sideways"), "{reason}");
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let spec = parse(
        "# a comment\n\nname Quiet\n\nkill_y 0  # trailing comment\nspawn 0 1 0\nbox 0 0 0 0 0 0 \
         minecraft:stone\n",
    )
    .expect("comments should not be directives");
    assert_eq!(spec.name, "Quiet");
    assert_eq!(spec.spawns.len(), 1);
}

/// A name runs to the end of the line, so a map can be called "Sky Fortress"
/// without quoting.
#[test]
fn a_name_may_contain_spaces() {
    let spec = parse("name Sky Fortress\nkill_y 0\nspawn 0 1 0\n").expect("a two-word name parses");
    assert_eq!(spec.name, "Sky Fortress");
}

// --- the brushes -----------------------------------------------------------

fn covered(brush: Brush) -> HashSet<[i32; 3]> {
    let mut out = HashSet::new();
    brush.each_block(|at, _| {
        out.insert(at);
    });
    out
}

/// Both corners inclusive, which is how the map files are written.
#[test]
fn a_box_covers_both_corners() {
    let blocks = covered(Brush::Box {
        min: [0, 0, 0],
        max: [1, 2, 3],
        block: "minecraft:stone",
    });
    assert_eq!(blocks.len(), 2 * 3 * 4);
    assert!(blocks.contains(&[0, 0, 0]));
    assert!(blocks.contains(&[1, 2, 3]));
}

/// Corners in either order, because a builder writing a box backwards should
/// get a box rather than nothing.
#[test]
fn a_box_does_not_care_which_corner_comes_first() {
    let forwards = covered(Brush::Box {
        min: [0, 0, 0],
        max: [2, 2, 2],
        block: "minecraft:stone",
    });
    let backwards = covered(Brush::Box {
        min: [2, 2, 2],
        max: [0, 0, 0],
        block: "minecraft:stone",
    });
    assert_eq!(forwards, backwards);
}

#[test]
fn a_cylinder_is_round_and_starts_at_its_centre() {
    let blocks = covered(Brush::Cylinder {
        centre: [0, 64, 0],
        radius: 4,
        height: 2,
        block: "minecraft:stone",
    });
    // The base is at the centre's own y and the height grows upwards.
    assert!(blocks.contains(&[0, 64, 0]));
    assert!(blocks.contains(&[0, 65, 0]));
    assert!(!blocks.contains(&[0, 63, 0]));
    // Round, not square: the corner of the bounding box is outside it.
    assert!(blocks.contains(&[4, 64, 0]));
    assert!(!blocks.contains(&[4, 64, 4]));
    assert!(!blocks.contains(&[5, 64, 0]));
}

#[test]
fn a_sphere_is_centred_on_its_centre() {
    let blocks = covered(Brush::Sphere {
        centre: [0, 0, 0],
        radius: 2,
        block: "minecraft:stone",
    });
    assert!(blocks.contains(&[0, 0, 0]));
    for axis in 0..3 {
        let mut at = [0, 0, 0];
        at[axis] = 2;
        assert!(blocks.contains(&at), "sphere should reach {at:?}");
        at[axis] = -2;
        assert!(blocks.contains(&at), "sphere should reach {at:?}");
    }
    assert!(!blocks.contains(&[2, 2, 2]));
}

/// A cone tapers downwards from the centre, which is what puts an underside on
/// a floating island rather than a flat slab.
#[test]
fn a_cone_tapers_downwards_to_a_point() {
    let depth = 8;
    let blocks = covered(Brush::Cone {
        centre: [0, 64, 0],
        radius: 6,
        depth,
        block: "minecraft:stone",
    });
    let width = |y: i32| {
        blocks
            .iter()
            .filter(|at| at[1] == y)
            .map(|at| at[0])
            .max()
            .unwrap_or(-1)
    };
    assert_eq!(width(64), 6, "the top level should be the full radius");
    assert!(
        width(64 - depth + 1) < width(64),
        "the bottom level should be narrower than the top"
    );
    // Downwards only: nothing above the centre.
    assert!(!blocks.iter().any(|at| at[1] > 64));
    // And nothing below the depth it was given.
    assert!(!blocks.iter().any(|at| at[1] <= 64 - depth));
}
