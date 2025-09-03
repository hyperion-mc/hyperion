use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct PickFromInventoryC2s {
    pub slot_to_use: VarInt,
}
