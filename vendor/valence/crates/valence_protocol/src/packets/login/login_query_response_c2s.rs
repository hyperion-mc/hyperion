use crate::{Bounded, DecodeBytes, Encode, Packet, PacketState, RawBytes, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginQueryResponseC2s<'a> {
    pub message_id: VarInt,
    pub data: Option<Bounded<RawBytes<'a>, 1048576>>,
}
