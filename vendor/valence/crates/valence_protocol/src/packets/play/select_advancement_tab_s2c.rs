use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct SelectAdvancementTabS2c {
    pub identifier: Option<Ident>,
}
