use glam::I16Vec2;
use rkyv::{Archive, Deserialize, Serialize};

/// Position of a chunk in the world
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
#[rkyv(derive(Debug))]
pub struct ChunkPosition {
    /// X coordinate of the chunk
    pub x: i16,
    /// Z coordinate of the chunk 
    pub z: i16,
}

impl ChunkPosition {
    /// Creates a new chunk position from x and z coordinates
    #[must_use]
    pub const fn new(x: i16, z: i16) -> Self {
        Self { x, z }
    }
}

impl From<I16Vec2> for ChunkPosition {
    fn from(value: I16Vec2) -> Self {
        Self {
            x: value.x,
            z: value.y,
        }
    }
}
