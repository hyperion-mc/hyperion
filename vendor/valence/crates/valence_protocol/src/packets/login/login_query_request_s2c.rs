use valence_ident::Ident;

use crate::{Bounded, DecodeBytes, Encode, Packet, PacketState, RawBytes, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginQueryRequestS2c<'a> {
    pub message_id: VarInt,
    pub channel: Ident,
    pub data: Bounded<RawBytes<'a>, 1048576>,
}
