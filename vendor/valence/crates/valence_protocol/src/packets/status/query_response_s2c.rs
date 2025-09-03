use valence_bytes::CowUtf8Bytes;

use crate::{DecodeBytes, Encode, Packet, PacketState};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Status)]
pub struct QueryResponseS2c<'a> {
    pub json: CowUtf8Bytes<'a>,
}
