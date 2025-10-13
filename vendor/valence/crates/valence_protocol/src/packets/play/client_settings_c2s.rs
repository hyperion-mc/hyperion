use bitfield_struct::bitfield;
use valence_bytes::CowUtf8Bytes;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ClientSettingsC2s<'a> {
    pub locale: CowUtf8Bytes<'a>,
    pub view_distance: u8,
    pub chat_mode: ChatMode,
    pub chat_colors: bool,
    pub displayed_skin_parts: DisplayedSkinParts,
    pub main_arm: MainArm,
    pub enable_text_filtering: bool,
    pub allow_server_listings: bool,
}

#[bitfield(u8)]
#[derive(PartialEq, Eq, Encode, Decode, DecodeBytesAuto)]
pub struct DisplayedSkinParts {
    pub cape: bool,
    pub jacket: bool,
    pub left_sleeve: bool,
    pub right_sleeve: bool,
    pub left_pants_leg: bool,
    pub right_pants_leg: bool,
    pub hat: bool,
    _pad: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug, Encode, Decode, DecodeBytes)]
pub enum ChatMode {
    Enabled,
    CommandsOnly,
    #[default]
    Hidden,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Encode, Decode, DecodeBytes)]
pub enum MainArm {
    Left,
    #[default]
    Right,
}
