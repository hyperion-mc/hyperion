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
        ability::Ability,
        effect::Effect,
        kit::Kit,
        player::Player,
        projectile::Projectile,
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
        ("Kit", world.component::<Kit>().has(id::<flecs::Final>())),
        (
            "Ability",
            world.component::<Ability>().has(id::<flecs::Final>()),
        ),
        (
            "Effect",
            world.component::<Effect>().has(id::<flecs::Final>()),
        ),
        (
            "Projectile",
            world.component::<Projectile>().has(id::<flecs::Final>()),
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
        "isa_kit" => {
            world.entity().is_a(id::<Kit>());
        }
        "isa_ability" => {
            world.entity().is_a(id::<Ability>());
        }
        "isa_effect" => {
            world.entity().is_a(id::<Effect>());
        }
        "isa_projectile" => {
            world.entity().is_a(id::<Projectile>());
        }
        other => panic!("unknown violation case: {other}"),
    }
    eprintln!("NO_ABORT");
    std::process::exit(0);
}

/// The child imports a full smash world, whose `ecs_init` segfaults on the
/// order of 1 in 100 runs -- ENG-10852, a pre-existing flake unrelated to the
/// trait under test. Such a crash kills the child before it reaches the illegal
/// operation, producing neither `CONSTRAINT_VIOLATED` nor `NO_ABORT`: a silent
/// non-zero death. Only that no-verdict shape is retried. `NO_ABORT` (the add
/// was accepted -- guard broken) fails at once and `CONSTRAINT_VIOLATED` (the
/// guard held) passes, so the retry masks neither verdict; and if every attempt
/// crashes with no verdict the assertion still fails.
const CONSTRAINT_ATTEMPTS: usize = 8;

fn assert_aborts_on_constraint(case: &str) {
    let exe = std::env::current_exe().expect("test binary path");
    for _ in 0..CONSTRAINT_ATTEMPTS {
        let output = Command::new(&exe)
            .args(["--exact", "violation_child", "--nocapture"])
            .env("SMASH_FINAL_VIOLATION", case)
            .env("RUST_BACKTRACE", "0")
            .output()
            .expect("spawn the violation child");
        let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
        log.push_str(&String::from_utf8_lossy(&output.stderr));
        assert!(
            !log.contains("NO_ABORT") && !output.status.success(),
            "{case}: inheriting from a Final kind was accepted, not aborted.\noutput:\n{log}"
        );
        if log.contains("CONSTRAINT_VIOLATED") {
            return;
        }
        // No verdict: the child died in ecs_init (ENG-10852), not on the is_a. Retry.
    }
    panic!(
        "{case}: the violation child crashed before reaching the constraint on all \
         {CONSTRAINT_ATTEMPTS} attempts -- ecs_init failing every time, or the trait no longer \
         aborts."
    );
}

#[test]
#[ignore = "behavioural subprocess-abort: the re-exec child crashes in ecs_init on the linux CI \
            runner at a high rate, ENG-10852/ENG-10951. The structural checks above gate the trait \
            in CI; run this locally with --run-ignored."]
fn inheriting_from_a_leaf_kind_aborts() {
    for case in [
        "isa_player",
        "isa_podium",
        "isa_kit",
        "isa_ability",
        "isa_effect",
        "isa_projectile",
    ] {
        assert_aborts_on_constraint(case);
    }
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
