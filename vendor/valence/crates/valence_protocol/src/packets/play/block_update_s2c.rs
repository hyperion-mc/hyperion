use crate::{BlockPos, BlockState, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct BlockUpdateS2c {
    pub position: BlockPos,
    pub block_id: BlockState,
}
