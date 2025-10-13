use bitfield_struct::bitfield;
use valence_bytes::CowUtf8Bytes;

use crate::{BlockPos, Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct UpdateCommandBlockC2s<'a> {
    pub position: BlockPos,
    pub command: CowUtf8Bytes<'a>,
    pub mode: UpdateCommandBlockMode,
    pub flags: UpdateCommandBlockFlags,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum UpdateCommandBlockMode {
    Sequence,
    Auto,
    Redstone,
}

#[bitfield(u8)]
#[derive(PartialEq, Eq, Encode, Decode, DecodeBytesAuto)]
pub struct UpdateCommandBlockFlags {
    pub track_output: bool,
    pub conditional: bool,
    pub automatic: bool,
    #[bits(5)]
    _pad: u8,
}
