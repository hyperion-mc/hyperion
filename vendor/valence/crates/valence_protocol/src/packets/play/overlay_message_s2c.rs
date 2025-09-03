use std::borrow::Cow;

use valence_text::Text;

use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct OverlayMessageS2c<'a> {
    pub action_bar_text: Cow<'a, Text>,
}
