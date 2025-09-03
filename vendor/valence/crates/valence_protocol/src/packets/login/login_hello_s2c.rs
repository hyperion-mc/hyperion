use valence_bytes::{CowBytes, CowUtf8Bytes};

use crate::{Bounded, DecodeBytes, Encode, Packet, PacketState};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginHelloS2c<'a> {
    pub server_id: Bounded<CowUtf8Bytes<'a>, 20>,
    pub public_key: CowBytes<'a>,
    pub verify_token: CowBytes<'a>,
}
