use std::borrow::Cow;

use uuid::Uuid;

use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct PlayerRemoveS2c<'a> {
    pub uuids: Cow<'a, [Uuid]>,
}
