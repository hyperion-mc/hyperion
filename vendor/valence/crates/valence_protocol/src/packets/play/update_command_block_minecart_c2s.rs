use valence_bytes::CowUtf8Bytes;

use crate::{DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct UpdateCommandBlockMinecartC2s<'a> {
    pub entity_id: VarInt,
    pub command: CowUtf8Bytes<'a>,
    pub track_output: bool,
}
