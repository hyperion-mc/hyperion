use std::borrow::Cow;

use valence_bytes::CowUtf8Bytes;
use valence_text::Text;

use crate::{DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct CommandSuggestionsS2c<'a> {
    pub id: VarInt,
    pub start: VarInt,
    pub length: VarInt,
    pub matches: Vec<CommandSuggestionsMatch<'a>>,
}

#[derive(Clone, PartialEq, Debug, Encode, DecodeBytes)]
pub struct CommandSuggestionsMatch<'a> {
    pub suggested_match: CowUtf8Bytes<'a>,
    pub tooltip: Option<Cow<'a, Text>>,
}
