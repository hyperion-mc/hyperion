use crate::{Decode, DecodeBytesAuto, Encode, Packet, PacketState, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginCompressionS2c {
    pub threshold: VarInt,
}
