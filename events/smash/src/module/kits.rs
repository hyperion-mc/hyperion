//! Every kit, as an import list.
//!
//! This file is the *only* place in the crate that names a kit, and all it does
//! is import modules. Nothing dispatches on kit identity, so a fifth kit is a
//! fifth file and a fifth line. `tests/modularity.rs` proves it by defining a
//! kit outside the crate entirely and never touching this list.

use flecs_ecs::prelude::*;

pub mod blaze;
pub mod chicken;
pub mod cow;
pub mod creeper;
pub mod enderman;
pub mod guardian;
pub mod iron_golem;
pub mod skeleton;
pub mod sky_squid;
pub mod slime;
pub mod snowman;
pub mod spider;
pub mod wither_skeleton;
pub mod wolf;
pub mod zombie;

/// Imports the kits that ship with the game.
#[derive(Component)]
pub struct StockKits;

impl Module for StockKits {
    fn module(world: &World) {
        world.module::<Self>("smash::kits");

        // The four Mineplex gave away, in the order its own kit menu did.
        world.import::<skeleton::Skeleton>();
        world.import::<iron_golem::IronGolem>();
        world.import::<spider::Spider>();
        world.import::<slime::Slime>();

        // The gem kits, cheapest first, which is also the order they unlocked.
        world.import::<enderman::Enderman>();
        world.import::<sky_squid::SkySquid>();
        world.import::<creeper::Creeper>();
        world.import::<wolf::Wolf>();
        world.import::<snowman::Snowman>();
        world.import::<wither_skeleton::WitherSkeleton>();
        world.import::<zombie::Zombie>();
        world.import::<cow::Cow>();
        world.import::<blaze::Blaze>();
        world.import::<chicken::Chicken>();
        world.import::<guardian::Guardian>();
    }
}
