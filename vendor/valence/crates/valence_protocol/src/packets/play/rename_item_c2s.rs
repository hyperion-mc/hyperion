use valence_bytes::CowUtf8Bytes;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct RenameItemC2s<'a> {
    // Surprisingly, this is not bounded as of 1.20.1.
    pub item_name: CowUtf8Bytes<'a>,
}
