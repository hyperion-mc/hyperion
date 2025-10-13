use valence_bytes::CowUtf8Bytes;
use valence_ident::Ident;

use crate::{BlockPos, DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct UpdateJigsawC2s<'a> {
    pub position: BlockPos,
    pub name: Ident,
    pub target: Ident,
    pub pool: Ident,
    pub final_state: CowUtf8Bytes<'a>,
    pub joint_type: CowUtf8Bytes<'a>,
}
