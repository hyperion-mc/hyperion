use valence_bytes::CowUtf8Bytes;

use crate::{Bounded, DecodeBytes, Encode, Packet, VarInt};

pub const MAX_TITLE_CHARS: usize = 128;
pub const MAX_PAGE_CHARS: usize = 8192;
pub const MAX_PAGES: usize = 200;

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct BookUpdateC2s<'a> {
    pub slot: VarInt,
    pub entries: Bounded<Vec<Bounded<CowUtf8Bytes<'a>, MAX_PAGE_CHARS>>, MAX_PAGES>,
    pub title: Option<Bounded<CowUtf8Bytes<'a>, MAX_TITLE_CHARS>>,
}
