use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct WorldBorderWarningBlocksChangedS2c {
    pub warning_blocks: VarInt,
}
