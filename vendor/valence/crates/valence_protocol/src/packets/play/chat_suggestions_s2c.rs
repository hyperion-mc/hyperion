use std::borrow::Cow;

use valence_bytes::CowUtf8Bytes;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ChatSuggestionsS2c<'a> {
    pub action: ChatSuggestionsAction,
    pub entries: Cow<'a, [CowUtf8Bytes<'a>]>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum ChatSuggestionsAction {
    Add,
    Remove,
    Set,
}
