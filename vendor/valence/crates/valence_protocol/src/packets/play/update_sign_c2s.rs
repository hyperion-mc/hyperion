use valence_bytes::CowUtf8Bytes;

use crate::{BlockPos, Bounded, DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct UpdateSignC2s<'a> {
    pub position: BlockPos,
    pub is_front_text: bool,
    pub lines: [Bounded<CowUtf8Bytes<'a>, 384>; 4],
}
