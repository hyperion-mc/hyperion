use std::borrow::Cow;

use bevy_ecs::prelude::*;
use valence_bytes::CowUtf8Bytes;
use valence_text::Text;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ScoreboardObjectiveUpdateS2c<'a> {
    pub objective_name: CowUtf8Bytes<'a>,
    pub mode: ObjectiveMode<'a>,
}

#[derive(Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum ObjectiveMode<'a> {
    Create {
        objective_display_name: Cow<'a, Text>,
        render_type: ObjectiveRenderType,
    },
    Remove,
    Update {
        objective_display_name: Cow<'a, Text>,
        render_type: ObjectiveRenderType,
    },
}

#[derive(
    Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto, Component, Default,
)]
pub enum ObjectiveRenderType {
    /// Display the value as a number.
    #[default]
    Integer,
    /// Display the value as hearts.
    Hearts,
}
