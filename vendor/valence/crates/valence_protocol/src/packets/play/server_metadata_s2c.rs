use std::borrow::Cow;

use valence_bytes::CowBytes;
use valence_text::Text;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ServerMetadataS2c<'a> {
    pub motd: Cow<'a, Text>,
    pub icon: Option<CowBytes<'a>>,
    pub enforce_secure_chat: bool,
}
