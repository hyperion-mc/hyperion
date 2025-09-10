use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntityAnimationS2c {
    pub entity_id: VarInt,
    pub animation: u8,
}
