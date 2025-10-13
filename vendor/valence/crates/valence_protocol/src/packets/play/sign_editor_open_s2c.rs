use crate::{BlockPos, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct SignEditorOpenS2c {
    pub location: BlockPos,
}
