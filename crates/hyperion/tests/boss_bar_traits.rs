//! `ShownTo` and `Sent` are relationships, proven the same two ways as the smash
//! relations in `events/smash/tests/relationship_traits.rs`: a structural check
//! that the module declared the trait, and a subprocess check that a bare-tag
//! `add` really aborts. flecs enforces the constraint with `abort()` (SIGABRT),
//! not a catchable panic, so the behavioural half runs in a child process.

use std::process::Command;

use flecs_ecs::prelude::*;
use hyperion::{
    HyperionCore,
    egress::boss_bar::{Sent, ShownTo},
};

/// A world with hyperion's real module declarations in place. `HyperionCore`
/// imports `EgressModule`, which imports `BossBarModule`, so the boss-bar
/// relationship traits are applied here exactly as they are on a server.
fn core() -> World {
    let world = World::new();
    world.import::<HyperionCore>();
    world
}

#[test]
fn boss_bar_edges_are_declared_relationships() {
    let world = core();
    assert!(
        world.component::<ShownTo>().has(id::<flecs::Relationship>()),
        "ShownTo lost its `flecs::Relationship` trait"
    );
    assert!(
        world.component::<Sent>().has(id::<flecs::Relationship>()),
        "Sent lost its `flecs::Relationship` trait"
    );
}

/// The child half: with `HYPERION_VIOLATION` set, add a boss-bar relationship as
/// a bare tag, which flecs aborts on. Unset -- every normal run -- it is a
/// passing no-op.
#[test]
fn violation_child() {
    let Ok(which) = std::env::var("HYPERION_VIOLATION") else {
        return;
    };
    let world = core();
    match which.as_str() {
        "shownto_bare" => {
            world.entity().add(id::<ShownTo>());
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
        .env("HYPERION_VIOLATION", case)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("spawn the violation child");
    // flecs writes its abort diagnostic to stdout; NO_ABORT goes to stderr.
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        !output.status.success(),
        "{case}: the bare-tag add was accepted, not aborted.\noutput:\n{log}"
    );
    assert!(
        !log.contains("NO_ABORT"),
        "{case}: flecs accepted the illegal state.\noutput:\n{log}"
    );
    assert!(
        log.contains("CONSTRAINT_VIOLATED"),
        "{case}: the child died, but not on a flecs constraint.\noutput:\n{log}"
    );
}

/// `ShownTo` is a tag relationship, so a bare-tag `add` compiles and flecs
/// aborts on it. `Sent` carries pair data, so `add(id::<Sent>())` does not even
/// compile -- the crate's non-ZST assertion refuses it before flecs would --
/// which is why only `ShownTo` has a behavioural case here. `Sent`'s trait is
/// covered by the structural check above; there is no bare-tag spelling of it
/// left for a caller to reach.
#[test]
fn a_boss_bar_edge_added_as_a_bare_tag_aborts() {
    assert_aborts_on_constraint("shownto_bare");
}
