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
        world
            .component::<ShownTo>()
            .has(id::<flecs::Relationship>()),
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

/// The child imports a full hyperion world, whose `ecs_init` segfaults on the
/// order of 1 in 100 runs -- a pre-existing flake, ENG-10852, unrelated to the
/// trait under test. A segfault kills the child before it reaches the bare-tag
/// add, so it produces neither the flecs `CONSTRAINT_VIOLATED` diagnostic nor
/// the `NO_ABORT` marker: a silent non-zero death with empty output. That is
/// exactly the shape we retry, and only that shape.
///
/// The two verdicts that end the loop are both decisive and neither is masked
/// by the retry: `NO_ABORT` means flecs *accepted* the illegal add (the guard
/// is broken) and fails at once; `CONSTRAINT_VIOLATED` means it aborted on the
/// constraint (the guard holds) and passes. Only a no-verdict crash is retried,
/// and if every attempt crashes without a verdict the assertion still fails --
/// so a child that could never initialise surfaces as a failure, it is not
/// swept under the retry.
const CONSTRAINT_ATTEMPTS: usize = 8;

fn assert_aborts_on_constraint(case: &str) {
    let exe = std::env::current_exe().expect("test binary path");
    let mut crashes = 0usize;
    for _ in 0..CONSTRAINT_ATTEMPTS {
        let output = Command::new(&exe)
            .args(["--exact", "violation_child", "--nocapture"])
            .env("HYPERION_VIOLATION", case)
            .env("RUST_BACKTRACE", "0")
            .output()
            .expect("spawn the violation child");
        // flecs writes its abort diagnostic to stdout; NO_ABORT goes to stderr.
        let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
        log.push_str(&String::from_utf8_lossy(&output.stderr));

        assert!(
            !log.contains("NO_ABORT") && !output.status.success(),
            "{case}: the bare-tag add was accepted, not aborted.\noutput:\n{log}"
        );
        if log.contains("CONSTRAINT_VIOLATED") {
            return;
        }
        // No verdict either way: the child died before reaching the add, which
        // is the ENG-10852 `ecs_init` segfault, not the trait. Retry.
        crashes += 1;
    }
    panic!(
        "{case}: the violation child crashed before reaching the constraint on all \
         {CONSTRAINT_ATTEMPTS} attempts ({crashes} no-verdict deaths). Either ecs_init is failing \
         every time (not the ~1/100 ENG-10852 flake) or the trait no longer aborts."
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
