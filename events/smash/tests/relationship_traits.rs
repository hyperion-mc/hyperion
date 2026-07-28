//! The relationship traits, and proof they refuse the states they forbid.
//!
//! `flecs::Relationship` and `(flecs::OneOf, LifeTier)` do not make an invalid
//! write unlikely; they make it a `CONSTRAINT_VIOLATED` abort at the call site.
//! That abort is a process `abort()` (SIGABRT), not a Rust panic, so it cannot
//! be caught with `#[should_panic]` -- it would take the whole test binary down.
//! The proof therefore comes in two halves:
//!
//! * **Structural, in-process.** Each relation carries the trait its module
//!   declared. This is the guard that fails the moment someone deletes a trait
//!   from a module, and it is load-immune -- it reads the world, it does not
//!   race a clock.
//! * **Behavioural, in a subprocess.** A representative violation of each trait
//!   really aborts, with `CONSTRAINT_VIOLATED` on stderr. This is what proves
//!   the trait has teeth rather than merely being present. It runs the offending
//!   `add` in a child process and asserts the child died on the constraint.

use std::process::Command;

use flecs_ecs::prelude::*;
use glam::Vec3;
use smash::{
    SmashModule,
    module::{
        ability::Grants,
        effect::{self, Affliction, Blame, Effect, InflictedBy, Source, Upon},
        kit::Playing,
        lives::{LifeTier, ShownAs},
        projectile::{self, FiredBy, Flight, Payload, Projectile},
        selector::{Offers, StandsOn},
    },
};

/// A game world with every relation's real trait declaration in place.
fn game() -> World {
    let world = World::new();
    world.import::<SmashModule>();
    world
}

// --- Structural: the module declared the trait ---------------------------

#[test]
fn relations_are_declared_relationships() {
    let world = game();
    for (name, present) in [
        ("ShownAs", world.component::<ShownAs>().has(id::<flecs::Relationship>())),
        ("Offers", world.component::<Offers>().has(id::<flecs::Relationship>())),
        ("StandsOn", world.component::<StandsOn>().has(id::<flecs::Relationship>())),
        ("Playing", world.component::<Playing>().has(id::<flecs::Relationship>())),
        ("Grants", world.component::<Grants>().has(id::<flecs::Relationship>())),
        ("Upon", world.component::<Upon>().has(id::<flecs::Relationship>())),
        ("InflictedBy", world.component::<InflictedBy>().has(id::<flecs::Relationship>())),
        ("Source", world.component::<Source>().has(id::<flecs::Relationship>())),
        ("FiredBy", world.component::<FiredBy>().has(id::<flecs::Relationship>())),
    ] {
        assert!(present, "{name} lost its `flecs::Relationship` trait");
    }
}

/// The two player-targeted relations are stored sparsely and delete their
/// source with their target. `DontFragment` keeps a volley of arrows or a
/// stream of burns from minting one archetype per victim; `(OnDeleteTarget,
/// Delete)` is what lets `tick_effects` and the fly system carry no
/// victimless/ownerless cleanup branch of their own.
#[test]
fn player_targeted_relations_are_sparse_and_cascade() {
    let world = game();
    for (name, dont_fragment, cascades) in [
        (
            "Upon",
            world.component::<Upon>().has(id::<flecs::DontFragment>()),
            world
                .component::<Upon>()
                .has((id::<flecs::OnDeleteTarget>(), id::<flecs::Delete>())),
        ),
        (
            "FiredBy",
            world.component::<FiredBy>().has(id::<flecs::DontFragment>()),
            world
                .component::<FiredBy>()
                .has((id::<flecs::OnDeleteTarget>(), id::<flecs::Delete>())),
        ),
    ] {
        assert!(dont_fragment, "{name} lost its `flecs::DontFragment` trait");
        assert!(cascades, "{name} lost its `(OnDeleteTarget, Delete)` trait");
    }
}

#[test]
fn shownas_targets_are_confined_to_life_tiers() {
    let world = game();
    assert!(
        world
            .component::<ShownAs>()
            .has((id::<flecs::OneOf>(), id::<LifeTier>())),
        "ShownAs lost its `(OneOf, LifeTier)` trait"
    );
}

// --- Behavioural: a violation really aborts ------------------------------

/// The child half of the subprocess guards. When `SMASH_VIOLATION` names a
/// case, it performs that illegal `add`, which flecs aborts on. If flecs did
/// *not* abort, this returns and the child exits 0, which the parent reads as a
/// missing guard. With the variable unset -- every normal `cargo test` run --
/// it is an ordinary passing no-op.
#[test]
fn violation_child() {
    let Ok(which) = std::env::var("SMASH_VIOLATION") else {
        return;
    };
    let world = game();
    match which.as_str() {
        // A relationship used as a bare tag.
        "shownas_bare" => {
            world.entity().add(id::<ShownAs>());
        }
        "offers_bare" => {
            world.entity().add(id::<Offers>());
        }
        "standson_bare" => {
            world.entity().add(id::<StandsOn>());
        }
        "playing_bare" => {
            world.entity().add(id::<Playing>());
        }
        "grants_bare" => {
            world.entity().add(id::<Grants>());
        }
        "upon_bare" => {
            world.entity().add(id::<Upon>());
        }
        "inflictedby_bare" => {
            world.entity().add(id::<InflictedBy>());
        }
        "source_bare" => {
            world.entity().add(id::<Source>());
        }
        "firedby_bare" => {
            world.entity().add(id::<FiredBy>());
        }
        // A `(ShownAs, x)` whose target is not a life tier.
        "shownas_wrong_target" => {
            let stray = world.entity_named("not_a_tier");
            world.entity().add((id::<ShownAs>(), stray));
        }
        other => panic!("unknown violation case: {other}"),
    }
    // Reached only if flecs accepted the illegal state.
    eprintln!("NO_ABORT");
    std::process::exit(0);
}

/// Run `violation_child` in a subprocess with `case` selected, and assert it
/// died on a flecs constraint.
fn assert_aborts_on_constraint(case: &str) {
    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(exe)
        .args(["--exact", "violation_child", "--nocapture"])
        .env("SMASH_VIOLATION", case)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("spawn the violation child");
    // flecs writes its `abort()` diagnostic to stdout; the child's own
    // `NO_ABORT` marker goes to stderr. Read both.
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        !output.status.success(),
        "{case}: the illegal add was accepted, not aborted.\noutput:\n{log}"
    );
    assert!(
        !log.contains("NO_ABORT"),
        "{case}: flecs accepted the illegal state before aborting elsewhere.\noutput:\n{log}"
    );
    assert!(
        log.contains("CONSTRAINT_VIOLATED"),
        "{case}: the child died, but not on a flecs constraint.\noutput:\n{log}"
    );
}

#[test]
fn a_relationship_added_as_a_bare_tag_aborts() {
    for case in [
        "shownas_bare",
        "offers_bare",
        "standson_bare",
        "playing_bare",
        "grants_bare",
        "upon_bare",
        "inflictedby_bare",
        "source_bare",
        "firedby_bare",
    ] {
        assert_aborts_on_constraint(case);
    }
}

/// `(Upon, OnDeleteTarget, Delete)`: deleting the victim deletes the effect,
/// so there is nothing left for `tick_effects` to find victimless. Destructing
/// outside a system applies at once, so no tick is needed to observe it.
#[test]
fn an_effect_dies_with_its_victim() {
    let world = game();
    let attacker = world.entity_named("attacker");
    let victim = world.entity_named("victim");
    effect::afflict(
        (&world).into(),
        victim,
        Blame {
            source: attacker.id(),
            attacker: attacker.id(),
        },
        Affliction::shield(5.0),
    );
    assert_eq!(count::<Effect>(&world), 1, "the affliction was not created");

    victim.destruct();
    assert_eq!(
        count::<Effect>(&world),
        0,
        "the effect outlived the victim it was on"
    );
}

/// `(FiredBy, OnDeleteTarget, Delete)`: deleting the shooter deletes their
/// projectiles in flight, so the fly system never meets an ownerless one.
#[test]
fn a_projectile_dies_with_its_shooter() {
    let world = game();
    let shooter = world.entity_named("shooter");
    projectile::fire(
        (&world).into(),
        shooter,
        Flight {
            position: Vec3::ZERO,
            velocity: Vec3::X,
            gravity: 0.0,
            seconds_left: 10.0,
            radius: 0.4,
        },
        Payload::new(1.0, 0.0),
    );
    assert_eq!(count::<Projectile>(&world), 1, "the projectile was not fired");

    shooter.destruct();
    assert_eq!(
        count::<Projectile>(&world),
        0,
        "the projectile outlived the shooter who fired it"
    );
}

/// Count entities carrying `T`.
fn count<T: ComponentId>(world: &World) -> i32 {
    let mut n = 0;
    world
        .query::<()>()
        .with(T::id())
        .build()
        .each_entity(|_, ()| n += 1);
    n
}

#[test]
fn a_shownas_pointing_outside_the_tiers_aborts() {
    assert_aborts_on_constraint("shownas_wrong_target");
}
