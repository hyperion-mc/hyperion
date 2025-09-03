use crate::{Decode, DecodeBytesAuto, Encode, Packet, PacketState};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(state = PacketState::Status)]
pub struct QueryPongS2c {
    pub payload: u64,
}
