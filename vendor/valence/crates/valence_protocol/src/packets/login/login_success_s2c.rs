use std::borrow::Cow;

use uuid::Uuid;
use valence_bytes::CowUtf8Bytes;

use crate::profile::Property;
use crate::{Bounded, DecodeBytes, Encode, Packet, PacketState};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
#[packet(state = PacketState::Login)]
pub struct LoginSuccessS2c<'a> {
    pub uuid: Uuid,
    pub username: Bounded<CowUtf8Bytes<'a>, 16>,
    pub properties: Cow<'a, [Property<CowUtf8Bytes<'a>>]>,
}
