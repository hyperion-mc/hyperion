use std::borrow::Cow;

use valence_text::Text;

use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct TitleS2c<'a> {
    pub title_text: Cow<'a, Text>,
}
