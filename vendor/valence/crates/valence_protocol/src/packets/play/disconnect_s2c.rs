use std::borrow::Cow;

use valence_text::Text;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::DISCONNECT_S2C)]
pub struct DisconnectS2c<'a> {
    pub reason: Cow<'a, Text>,
}
