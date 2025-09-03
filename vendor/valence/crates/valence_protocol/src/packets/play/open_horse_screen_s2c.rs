use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct OpenHorseScreenS2c {
    pub window_id: u8,
    pub slot_count: VarInt,
    pub entity_id: i32,
}
