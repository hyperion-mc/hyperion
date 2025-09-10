use std::borrow::Cow;
use std::collections::BTreeSet;

use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct FeaturesS2c<'a> {
    pub features: Cow<'a, BTreeSet<Ident>>,
}
