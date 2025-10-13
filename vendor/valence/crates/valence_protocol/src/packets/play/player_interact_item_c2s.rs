use crate::{Decode, DecodeBytesAuto, Encode, Hand, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct PlayerInteractItemC2s {
    pub hand: Hand,
    pub sequence: VarInt,
}
