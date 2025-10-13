use valence_nbt::Compound;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct NbtQueryResponseS2c {
    pub transaction_id: VarInt,
    pub nbt: Compound,
}
