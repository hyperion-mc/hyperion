use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct CraftFailedResponseS2c {
    pub window_id: u8,
    pub recipe: Ident,
}
