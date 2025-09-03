use valence_ident::Ident;

use crate::{Bounded, DecodeBytes, Encode, Packet, RawBytes};

pub const MAX_PAYLOAD_SIZE: usize = 32767;

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct CustomPayloadC2s<'a> {
    pub channel: Ident,
    pub data: Bounded<RawBytes<'a>, MAX_PAYLOAD_SIZE>,
}
