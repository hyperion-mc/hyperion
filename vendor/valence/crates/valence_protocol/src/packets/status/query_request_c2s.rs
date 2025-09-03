use crate::{Decode, DecodeBytesAuto, Encode, Packet, PacketState};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(state = PacketState::Status)]
pub struct QueryRequestC2s;
