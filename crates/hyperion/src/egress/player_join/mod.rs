//! Startup state the world entry path depends on.
//!
//! The join sequence itself lives in [`crate::net::protocol::join`]. What is
//! left here is the two pieces of world state it and the spatial index need
//! before any player exists: the per-rayon-thread world stages, and the root of
//! the command tree.

use std::ops::Index;

use flecs_ecs::prelude::*;

mod list;
pub use list::*;

use crate::{
    simulation::command::{Command, ROOT_COMMAND},
    util::SendableRef,
};

#[derive(Component)]
pub struct PlayerJoinModule;

/// One flecs stage per rayon worker, so a parallel iteration can touch the
/// world without the stages aliasing.
#[derive(Component)]
pub struct RayonWorldStages {
    stages: Vec<SendableRef<'static>>,
}

impl Index<usize> for RayonWorldStages {
    type Output = WorldRef<'static>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.stages[index].0
    }
}

impl Module for PlayerJoinModule {
    fn module(world: &World) {
        let rayon_threads = rayon::current_num_threads();

        #[expect(
            clippy::unwrap_used,
            reason = "realistically, this should never fail; 2^31 is very large"
        )]
        let rayon_threads = i32::try_from(rayon_threads).unwrap();

        let stages = (0..rayon_threads)
            // SAFETY: promoting world to static lifetime, system won't outlive world
            .map(|i| unsafe { std::mem::transmute(world.stage(i)) })
            .map(SendableRef)
            .collect::<Vec<_>>();

        world
            .component::<RayonWorldStages>()
            .add_trait::<flecs::Singleton>();
        world.set(RayonWorldStages { stages });

        let root_command = world.entity().set(Command::ROOT);

        #[expect(
            clippy::unwrap_used,
            reason = "this is only called once on startup. We mostly care about crashing during \
                      server execution"
        )]
        ROOT_COMMAND.set(root_command.id()).unwrap();
    }
}
