use valence_ident::Ident;

use crate::block_pos::BlockPos;
use crate::{DecodeBytes, Encode};

#[derive(Clone, PartialEq, Eq, Debug, Encode, DecodeBytes)]
pub struct GlobalPos {
    pub dimension_name: Ident,
    pub position: BlockPos,
}
