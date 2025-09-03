use std::borrow::Cow;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, PacketState, Text};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginDisconnectS2c<'a> {
    pub reason: Cow<'a, Text>,
}
