//! `ShownTo` and `Sent` are relationships. This proves it structurally: the
//! module declares `flecs::Relationship` on each, so a bare-tag `add` becomes a
//! `CONSTRAINT_VIOLATED` abort at the call site rather than a silent no-op.
//!
//! There is deliberately no behavioural (subprocess) half here. The abort
//! itself is a property of flecs, not of these particular relations: any
//! `flecs::Relationship`-tagged component added as a bare tag aborts, and that
//! is already proven on the wire, on the same CI, by
//! `events/smash/tests/relationship_traits.rs` with a light `SmashModule`
//! child. A behavioural check here would need a child process that imports the
//! full `HyperionCore` (the boss-bar components' registration prerequisites
//! live there, so a bare `BossBarModule` import panics in flecs registration),
//! and that child crashes deterministically -- 8 of 8 -- in `ecs_init` under
//! the linux CI runner while passing on macOS. That is a real environment bug,
//! filed separately; a retry cannot clear a deterministic crash, and reddening
//! main to re-prove a flecs property already covered elsewhere is the wrong
//! trade. So this file proves the one thing that is boss-bar-specific -- that
//! `ShownTo`/`Sent` carry the trait -- and leaves the abort mechanism to the
//! test that can run it.

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
