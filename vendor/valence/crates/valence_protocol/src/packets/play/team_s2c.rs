use std::borrow::Cow;
use std::io::Write;

use anyhow::bail;
use bitfield_struct::bitfield;
use valence_bytes::{Bytes, CowUtf8Bytes, Utf8Bytes};
use valence_text::Text;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct TeamS2c<'a> {
    pub team_name: CowUtf8Bytes<'a>,
    pub mode: Mode<'a>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Mode<'a> {
    CreateTeam {
        team_display_name: Cow<'a, Text>,
        friendly_flags: TeamFlags,
        name_tag_visibility: NameTagVisibility,
        collision_rule: CollisionRule,
        team_color: TeamColor,
        team_prefix: Cow<'a, Text>,
        team_suffix: Cow<'a, Text>,
        entities: Vec<CowUtf8Bytes<'a>>,
    },
    RemoveTeam,
    UpdateTeamInfo {
        team_display_name: Cow<'a, Text>,
        friendly_flags: TeamFlags,
        name_tag_visibility: NameTagVisibility,
        collision_rule: CollisionRule,
        team_color: TeamColor,
        team_prefix: Cow<'a, Text>,
        team_suffix: Cow<'a, Text>,
    },
    AddEntities {
        entities: Vec<CowUtf8Bytes<'a>>,
    },
    RemoveEntities {
        entities: Vec<CowUtf8Bytes<'a>>,
    },
}

impl Encode for Mode<'_> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        match self {
            Mode::CreateTeam {
                team_display_name,
                friendly_flags,
                name_tag_visibility,
                collision_rule,
                team_color,
                team_prefix,
                team_suffix,
                entities,
            } => {
                0i8.encode(&mut w)?;
                team_display_name.encode(&mut w)?;
                friendly_flags.encode(&mut w)?;
                match name_tag_visibility {
                    NameTagVisibility::Always => "always",
                    NameTagVisibility::Never => "never",
                    NameTagVisibility::HideForOtherTeams => "hideForOtherTeams",
                    NameTagVisibility::HideForOwnTeam => "hideForOwnTeam",
                }
                .encode(&mut w)?;
                match collision_rule {
                    CollisionRule::Always => "always",
                    CollisionRule::Never => "never",
                    CollisionRule::PushOtherTeams => "pushOtherTeams",
                    CollisionRule::PushOwnTeam => "pushOwnTeam",
                }
                .encode(&mut w)?;
                team_color.encode(&mut w)?;
                team_prefix.encode(&mut w)?;
                team_suffix.encode(&mut w)?;
                entities.encode(&mut w)?;
            }
            Mode::RemoveTeam => 1i8.encode(&mut w)?,
            Mode::UpdateTeamInfo {
                team_display_name,
                friendly_flags,
                name_tag_visibility,
                collision_rule,
                team_color,
                team_prefix,
                team_suffix,
            } => {
                2i8.encode(&mut w)?;
                team_display_name.encode(&mut w)?;
                friendly_flags.encode(&mut w)?;
                match name_tag_visibility {
                    NameTagVisibility::Always => "always",
                    NameTagVisibility::Never => "never",
                    NameTagVisibility::HideForOtherTeams => "hideForOtherTeams",
                    NameTagVisibility::HideForOwnTeam => "hideForOwnTeam",
                }
                .encode(&mut w)?;
                match collision_rule {
                    CollisionRule::Always => "always",
                    CollisionRule::Never => "never",
                    CollisionRule::PushOtherTeams => "pushOtherTeams",
                    CollisionRule::PushOwnTeam => "pushOwnTeam",
                }
                .encode(&mut w)?;
                team_color.encode(&mut w)?;
                team_prefix.encode(&mut w)?;
                team_suffix.encode(&mut w)?;
            }
            Mode::AddEntities { entities } => {
                3i8.encode(&mut w)?;
                entities.encode(&mut w)?;
            }
            Mode::RemoveEntities { entities } => {
                4i8.encode(&mut w)?;
                entities.encode(&mut w)?;
            }
        }
        Ok(())
    }
}

impl<'a> DecodeBytes for Mode<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Ok(match i8::decode_bytes(r)? {
            0 => Self::CreateTeam {
                team_display_name: DecodeBytes::decode_bytes(r)?,
                friendly_flags: DecodeBytes::decode_bytes(r)?,
                name_tag_visibility: match Utf8Bytes::decode_bytes(r)?.as_ref() {
                    "always" => NameTagVisibility::Always,
                    "never" => NameTagVisibility::Never,
                    "hideForOtherTeams" => NameTagVisibility::HideForOtherTeams,
                    "hideForOwnTeam" => NameTagVisibility::HideForOwnTeam,
                    other => bail!("unknown name tag visibility type \"{other}\""),
                },
                collision_rule: match Utf8Bytes::decode_bytes(r)?.as_ref() {
                    "always" => CollisionRule::Always,
                    "never" => CollisionRule::Never,
                    "pushOtherTeams" => CollisionRule::PushOtherTeams,
                    "pushOwnTeam" => CollisionRule::PushOwnTeam,
                    other => bail!("unknown collision rule type \"{other}\""),
                },
                team_color: DecodeBytes::decode_bytes(r)?,
                team_prefix: DecodeBytes::decode_bytes(r)?,
                team_suffix: DecodeBytes::decode_bytes(r)?,
                entities: DecodeBytes::decode_bytes(r)?,
            },
            1 => Self::RemoveTeam,
            2 => Self::UpdateTeamInfo {
                team_display_name: DecodeBytes::decode_bytes(r)?,
                friendly_flags: DecodeBytes::decode_bytes(r)?,
                name_tag_visibility: match Utf8Bytes::decode_bytes(r)?.as_ref() {
                    "always" => NameTagVisibility::Always,
                    "never" => NameTagVisibility::Never,
                    "hideForOtherTeams" => NameTagVisibility::HideForOtherTeams,
                    "hideForOwnTeam" => NameTagVisibility::HideForOwnTeam,
                    other => bail!("unknown name tag visibility type \"{other}\""),
                },
                collision_rule: match Utf8Bytes::decode_bytes(r)?.as_ref() {
                    "always" => CollisionRule::Always,
                    "never" => CollisionRule::Never,
                    "pushOtherTeams" => CollisionRule::PushOtherTeams,
                    "pushOwnTeam" => CollisionRule::PushOwnTeam,
                    other => bail!("unknown collision rule type \"{other}\""),
                },
                team_color: DecodeBytes::decode_bytes(r)?,
                team_prefix: DecodeBytes::decode_bytes(r)?,
                team_suffix: DecodeBytes::decode_bytes(r)?,
            },
            3 => Self::AddEntities {
                entities: DecodeBytes::decode_bytes(r)?,
            },
            4 => Self::RemoveEntities {
                entities: DecodeBytes::decode_bytes(r)?,
            },
            n => bail!("unknown update teams action of {n}"),
        })
    }
}

#[bitfield(u8)]
#[derive(PartialEq, Eq, Encode, Decode, DecodeBytesAuto)]
pub struct TeamFlags {
    pub friendly_fire: bool,
    pub see_invisible_teammates: bool,
    #[bits(6)]
    _pad: u8,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NameTagVisibility {
    Always,
    Never,
    HideForOtherTeams,
    HideForOwnTeam,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CollisionRule {
    Always,
    Never,
    PushOtherTeams,
    PushOwnTeam,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum TeamColor {
    Black,
    DarkBlue,
    DarkGreen,
    DarkCyan,
    DarkRed,
    Purple,
    Gold,
    Gray,
    DarkGray,
    Blue,
    BrightGreen,
    Cyan,
    Red,
    Pink,
    Yellow,
    White,
    Obfuscated,
    Bold,
    Strikethrough,
    Underlined,
    Italic,
    Reset,
}
