use uuid::Uuid;
use valence_bytes::CowUtf8Bytes;

use crate::{Bounded, DecodeBytes, Encode, Packet, PacketState};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginHelloC2s<'a> {
    pub username: Bounded<CowUtf8Bytes<'a>, 16>,
    pub profile_id: Option<Uuid>,
}
