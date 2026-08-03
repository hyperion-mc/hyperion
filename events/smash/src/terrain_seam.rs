//! The host half of the block-world seam: hyperion's `Blocks` answering the
//! game's terrain reads.
//!
//! The mirror in `crate::mirror` copies host state onto game components once a
//! tick, which is the right shape for a player's position and the wrong one for
//! a world of blocks: there are millions of them, a tick looks at a handful, and
//! copying them would be maintaining a second authority that drifts the moment
//! anything places a block. So terrain is asked for rather than copied, and this
//! is the answering half.
//!
//! It is three lines of substance because the traversal is not here. Both sides
//! of the seam go through [`geometry::sweep::first_block_hit`] -- the block
//! store's own [`Blocks::first_collision`] is a wrapper over it, and so is the
//! [`crate::module::blocks::Cubes`] a test builds a wall from. One traversal,
//! two sources of shapes, which is what makes a test about a `Cubes` wall
//! evidence about a real one.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::blocks::Blocks;

use crate::module::blocks::{BlockHit, BlockWorld, BlockWorldComponentsModule, BlockWorldHandle};

/// hyperion's loaded chunks, as a [`BlockWorld`].
///
/// Carries no state: the block store is a singleton on the world handed to
/// [`BlockWorld::sweep`], because an `Arc<dyn BlockWorld>` outlives any borrow
/// of flecs storage and so cannot hold one.
#[derive(Debug, Default, Clone, Copy)]
pub struct HyperionBlocks;

impl BlockWorld for HyperionBlocks {
    fn sweep(&self, world: WorldRef<'_>, from: Vec3, to: Vec3) -> Option<BlockHit> {
        world.get::<&Blocks>(|blocks| {
            let hit = blocks.first_collision(geometry::ray::Ray::from_points(from, to))?;
            Some(BlockHit {
                time: hit.distance,
                block: hit.location,
                point: hit.point,
                normal: hit.normal,
                inside: hit.inside,
            })
        })
    }
}

/// Replaces the game half's `OpenAir` default with the real block store.
#[derive(Component)]
pub struct TerrainSeamModule;

impl Module for TerrainSeamModule {
    fn module(world: &World) {
        // The singleton this overwrites is registered by the game half, so the
        // module that registers it is imported rather than assumed: a `set` of
        // an unregistered component is an abort in a dev build and silence in a
        // release one.
        world.import::<BlockWorldComponentsModule>();
        world.set(BlockWorldHandle::new(HyperionBlocks));
    }
}
