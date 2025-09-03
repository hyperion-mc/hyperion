use crate::{ChunkPos, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct UnloadChunkS2c {
    pub pos: ChunkPos,
}
