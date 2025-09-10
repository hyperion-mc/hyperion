use std::borrow::Cow;

use valence_bytes::CowUtf8Bytes;
use valence_text::Text;

use crate::{Bounded, DecodeBytes, Encode, Packet};

#[derive(Clone, PartialEq, Debug, Encode, DecodeBytes, Packet)]
pub struct ResourcePackSendS2c<'a> {
    pub url: CowUtf8Bytes<'a>,
    pub hash: Bounded<CowUtf8Bytes<'a>, 40>,
    pub forced: bool,
    pub prompt_message: Option<Cow<'a, Text>>,
}
