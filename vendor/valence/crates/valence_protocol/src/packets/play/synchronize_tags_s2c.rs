use std::borrow::Cow;
use std::collections::BTreeMap;

use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct SynchronizeTagsS2c<'a> {
    pub groups: Cow<'a, RegistryMap>,
}

pub type RegistryMap = BTreeMap<Ident, BTreeMap<Ident, Vec<VarInt>>>;
