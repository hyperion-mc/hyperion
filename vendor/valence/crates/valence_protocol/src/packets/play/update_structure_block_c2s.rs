use bitfield_struct::bitfield;
use valence_bytes::CowUtf8Bytes;

use crate::{BlockPos, Bounded, Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet, VarLong};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct UpdateStructureBlockC2s<'a> {
    pub position: BlockPos,
    pub action: Action,
    pub mode: Mode,
    pub name: CowUtf8Bytes<'a>,
    pub offset_xyz: [i8; 3],
    pub size_xyz: [i8; 3],
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub metadata: Bounded<CowUtf8Bytes<'a>, 128>,
    pub integrity: f32,
    pub seed: VarLong,
    pub flags: Flags,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Action {
    UpdateData,
    SaveStructure,
    LoadStructure,
    DetectSize,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Mode {
    Save,
    Load,
    Corner,
    Data,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Mirror {
    None,
    LeftRight,
    FrontBack,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Rotation {
    None,
    Clockwise90,
    Clockwise180,
    Counterclockwise90,
}

#[bitfield(u8)]
#[derive(PartialEq, Eq, Encode, Decode, DecodeBytesAuto)]
pub struct Flags {
    pub ignore_entities: bool,
    pub show_air: bool,
    pub show_bounding_box: bool,
    #[bits(5)]
    _pad: u8,
}
