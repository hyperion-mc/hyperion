use glam::I16Vec2;
use rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
#[rkyv(derive(Debug))]
pub struct ChunkPosition {
    pub x: i16,
    pub z: i16,
}

impl ChunkPosition {
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

impl From<ChunkPosition> for I16Vec2 {
    fn from(value: ChunkPosition) -> Self {
        Self::new(value.x, value.z)
    }
}
