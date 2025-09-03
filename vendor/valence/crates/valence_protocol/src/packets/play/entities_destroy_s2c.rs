use std::borrow::Cow;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntitiesDestroyS2c<'a> {
    pub entity_ids: Cow<'a, [VarInt]>,
}
