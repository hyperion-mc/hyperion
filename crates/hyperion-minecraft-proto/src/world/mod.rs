//! World data: chunks, and the containers they are built out of.
//!
//! Everything here is transcribed from `net.minecraft.world.level.chunk` and
//! `net.minecraft.network.protocol.game`, which the packaged
//! `minecraft-decompiled` output does not cover -- it stops at
//! `net.minecraft.network`. Rebuild the wider set with cfr over
//! `net/minecraft/world/level/chunk` and `net/minecraft/util/SimpleBitStorage`
//! to check any of it.

pub mod chunk;
pub mod palette;

pub use chunk::{
    BlockEntity, ChunkData, ChunkSection, Heightmap, HeightmapKind, LIGHT_LAYER_LEN, LevelChunkWithLight,
    LightData, light_mask,
};
pub use palette::{ContainerKind, PalettedContainer, storage_len};
