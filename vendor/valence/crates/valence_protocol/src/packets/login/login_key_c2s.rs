use valence_bytes::CowBytes;

use crate::{DecodeBytes, Encode, Packet, PacketState};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginKeyC2s<'a> {
    pub shared_secret: CowBytes<'a>,
    pub verify_token: CowBytes<'a>,
}
