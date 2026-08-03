//! What the game is allowed to ask about the block world.
//!
//! The second seam, and the only read that crosses one. [`crate::server`] is
//! the write seam and carries no reads on purpose -- position, facing and
//! ground state arrive as mirrored components instead, because reading them is
//! a per-player-per-tick hot path and a mirror turns it into plain component
//! iteration. Terrain cannot be mirrored: it is millions of blocks, almost none
//! of which any tick looks at, and the one authoritative copy already exists on
//! the host. So this asks, rather than copies.
//!
//! Deliberately one method, and deliberately the *whole* question. A seam
//! spelled "is this block solid" would put the traversal on the game side and
//! cost a virtual call per cell; asking for the answer to the whole segment
//! costs one call per projectile per tick and leaves the traversal in
//! [`geometry::sweep`], where the block store and this crate's tests share it.
//!
//! The default is [`OpenAir`]. A world with [`crate::SmashModule`] and nothing
//! else -- which is every test under `tests/` and the whole of the mock -- has
//! no terrain, and saying so explicitly is what keeps the game half runnable
//! with no host anywhere near it.

use std::{collections::HashSet, sync::Arc};

use flecs_ecs::prelude::*;
use geometry::aabb::Aabb;
/// Where a swept segment first met a block.
///
/// Re-exported rather than redefined: the block store and this seam are
/// answering the same question, and a second struct with the same four fields
/// would be a conversion nobody reads and one field that eventually disagrees.
pub use geometry::sweep::BlockHit;
use glam::{IVec3, Vec3};

/// The block world, as the game sees it.
///
/// `world` is handed in rather than captured because the host's block store is
/// a flecs singleton: an `Arc<dyn BlockWorld>` outlives any borrow of flecs
/// storage, so the implementation looks the store up per call. Every caller has
/// a world in hand already, so this costs nothing at the call site.
pub trait BlockWorld: Send + Sync + 'static {
    /// The first block surface on the segment `from` -> `to`, or `None` if it
    /// is clear.
    ///
    /// A segment and not a point. A projectile is integrated a whole tick at a
    /// time and Barrage's arrows travel sixty blocks a second, so an endpoint
    /// test skips two of every three blocks on the path and a one-block wall is
    /// something an arrow flies through. The same reasoning that made
    /// `nearest_target` measure against a segment applies here, for the same
    /// reason.
    fn sweep(&self, world: WorldRef<'_>, from: Vec3, to: Vec3) -> Option<BlockHit>;
}

/// Nothing is solid. The default, and what every test that is not about terrain
/// gets.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAir;

impl BlockWorld for OpenAir {
    fn sweep(&self, _: WorldRef<'_>, _: Vec3, _: Vec3) -> Option<BlockHit> {
        None
    }
}

/// A block world of full cubes at the listed coordinates.
///
/// For tests, and for anything that wants terrain without a Minecraft server:
/// it answers through the same [`geometry::sweep::first_block_hit`] the host's
/// block store answers through, so a test built on it is evidence about the
/// shipped traversal rather than about a second copy of it. What it does not
/// carry is partial shapes -- everything here is a full cube.
#[derive(Debug, Default, Clone)]
pub struct Cubes(HashSet<IVec3>);

impl Cubes {
    #[must_use]
    pub fn new(solid: impl IntoIterator<Item = IVec3>) -> Self {
        Self(solid.into_iter().collect())
    }

    /// An axis-aligned wall filling the inclusive box between two corners.
    #[must_use]
    pub fn wall(min: IVec3, max: IVec3) -> Self {
        let mut solid = HashSet::new();
        for x in min.x..=max.x {
            for y in min.y..=max.y {
                for z in min.z..=max.z {
                    solid.insert(IVec3::new(x, y, z));
                }
            }
        }
        Self(solid)
    }
}

impl BlockWorld for Cubes {
    fn sweep(&self, _: WorldRef<'_>, from: Vec3, to: Vec3) -> Option<BlockHit> {
        geometry::sweep::first_block_hit(from, to, |block| {
            self.0
                .contains(&block)
                .then(|| Aabb::new(Vec3::ZERO, Vec3::ONE))
        })
    }
}

/// Singleton holding the live [`BlockWorld`].
///
/// The same shape as [`crate::server::ServerHandle`], for the same reason:
/// systems name `&BlockWorldHandle` as an ordinary query term and flecs
/// resolves it once per table rather than once per entity.
#[derive(Component)]
pub struct BlockWorldHandle(pub Arc<dyn BlockWorld>);

impl BlockWorldHandle {
    pub fn new(blocks: impl BlockWorld) -> Self {
        Self(Arc::new(blocks))
    }
}

impl core::ops::Deref for BlockWorldHandle {
    type Target = dyn BlockWorld;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// Registration module for the block-world seam: types only, no systems.
///
/// Installs [`OpenAir`] as the singleton's value in the same place the
/// singleton is registered, per the root `CLAUDE.md`: a bare `world.set` stores
/// a value without registering the type, which is an abort in a dev build and
/// silence in a release one.
#[derive(Component)]
pub struct BlockWorldComponentsModule;

impl Module for BlockWorldComponentsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Blocks");
        world
            .component::<BlockWorldHandle>()
            .add_trait::<flecs::Singleton>();
        world.set(BlockWorldHandle::new(OpenAir));
    }
}
