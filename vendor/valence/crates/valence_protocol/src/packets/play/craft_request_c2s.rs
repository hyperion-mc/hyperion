use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct CraftRequestC2s {
    pub window_id: i8,
    pub recipe: Ident,
    pub make_all: bool,
}
