use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct RecipeBookDataC2s {
    pub recipe_id: Ident,
}
