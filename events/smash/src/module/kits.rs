//! Every kit, as an import list.
//!
//! This file is the *only* place in the crate that names a kit, and all it does
//! is import modules. Nothing dispatches on kit identity, so a fifth kit is a
//! fifth file and a fifth line. `tests/modularity.rs` proves it by defining a
//! kit outside the crate entirely and never touching this list.

use flecs_ecs::prelude::*;

pub mod enderman;
pub mod iron_golem;
pub mod skeleton;
pub mod slime;

/// Imports the kits that ship with the game.
#[derive(Component)]
pub struct StockKits;

impl Module for StockKits {
    fn module(world: &World) {
        world.module::<Self>("smash::kits");
        world.import::<skeleton::Skeleton>();
        world.import::<iron_golem::IronGolem>();
        world.import::<enderman::Enderman>();
        world.import::<slime::Slime>();
    }
}
