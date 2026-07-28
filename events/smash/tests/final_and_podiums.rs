//! `Final` on the leaf entity kinds, and podiums as a named ring.
//!
//! `Final` asserts a kind is never inherited from: `is_a(Player)` or
//! `is_a(Podium)` becomes a `CONSTRAINT_VIOLATED` abort rather than a quiet
//! mistake. flecs enforces it with `abort()` (SIGABRT), so -- as in
//! `relationship_traits.rs` -- the proof is a structural check plus a
//! subprocess abort check. Naming is proven in-process: after the selector is
//! built, each podium is a named child of one `ring` parent.

use std::process::Command;

use flecs_ecs::prelude::*;
use glam::IVec3;
use smash::{
    SmashModule,
    module::{
        player::Player,
        selector::{self, Podium},
    },
};

fn game() -> World {
    let world = World::new();
    world.import::<SmashModule>();
    world
}

// --- Final: structural -----------------------------------------------------

#[test]
fn leaf_kinds_are_final() {
    let world = game();
    for (name, present) in [
        (
            "Player",
            world.component::<Player>().has(id::<flecs::Final>()),
        ),
        (
            "Podium",
            world.component::<Podium>().has(id::<flecs::Final>()),
        ),
    ] {
        assert!(present, "{name} lost its `flecs::Final` trait");
    }
}

// --- Final: behavioural (subprocess, because flecs aborts) -----------------

#[test]
fn violation_child() {
    let Ok(which) = std::env::var("SMASH_FINAL_VIOLATION") else {
        return;
    };
    let world = game();
    match which.as_str() {
        "isa_player" => {
            world.entity().is_a(id::<Player>());
        }
        "isa_podium" => {
            world.entity().is_a(id::<Podium>());
        }
        other => panic!("unknown violation case: {other}"),
    }
    eprintln!("NO_ABORT");
    std::process::exit(0);
}

fn assert_aborts_on_constraint(case: &str) {
    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(exe)
        .args(["--exact", "violation_child", "--nocapture"])
        .env("SMASH_FINAL_VIOLATION", case)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("spawn the violation child");
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        !output.status.success(),
        "{case}: inheriting from a Final kind was accepted, not aborted.\noutput:\n{log}"
    );
    assert!(
        !log.contains("NO_ABORT"),
        "{case}: flecs accepted the illegal is_a.\noutput:\n{log}"
    );
    assert!(
        log.contains("CONSTRAINT_VIOLATED"),
        "{case}: the child died, but not on a flecs constraint.\noutput:\n{log}"
    );
}

#[test]
fn inheriting_from_a_leaf_kind_aborts() {
    assert_aborts_on_constraint("isa_player");
    assert_aborts_on_constraint("isa_podium");
}

// --- Podiums: a named ring -------------------------------------------------

/// After the selector is built, every podium is a named child of one `ring`
/// parent, so the explorer shows `smash.Selector.ring.<Kit>` rather than a
/// scatter of bare ids.
#[test]
fn podiums_are_named_children_of_the_ring() {
    let world = game();
    selector::build(&world, IVec3::new(0, 64, 0));

    let ring = world
        .try_lookup("smash::Selector::ring")
        .expect("the selector built a `ring` parent under its module");

    let mut podiums = 0;
    let mut anonymous = Vec::new();
    let mut wrong_parent = Vec::new();
    world
        .query::<()>()
        .with(id::<Podium>())
        .build()
        .each_entity(|podium, ()| {
            podiums += 1;
            if podium.name().is_empty() {
                anonymous.push(podium.id());
            }
            if podium.parent().map(|p| p.id()) != Some(ring.id()) {
                wrong_parent.push(podium.id());
            }
        });

    assert!(podiums > 0, "the selector built no podiums");
    assert!(anonymous.is_empty(), "unnamed podiums: {anonymous:?}");
    assert!(
        wrong_parent.is_empty(),
        "podiums not parented to the ring: {wrong_parent:?}"
    );

    // The Skeleton podium reads at its qualified path, which is the whole point.
    assert!(
        world
            .try_lookup("smash::Selector::ring::Skeleton")
            .is_some_and(|e| e.has(id::<Podium>())),
        "the Skeleton podium is not at smash.Selector.ring.Skeleton"
    );
}
