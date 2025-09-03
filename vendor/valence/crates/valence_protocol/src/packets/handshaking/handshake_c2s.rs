use valence_bytes::CowUtf8Bytes;

use crate::{Bounded, Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet, PacketState, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Handshaking)]
pub struct HandshakeC2s<'a> {
    pub protocol_version: VarInt,
    pub server_address: Bounded<CowUtf8Bytes<'a>, 255>,
    pub server_port: u16,
    pub next_state: HandshakeNextState,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeBytesAuto)]
pub enum HandshakeNextState {
    #[packet(tag = 1)]
    Status,
    #[packet(tag = 2)]
    Login,
}
