use std::{borrow::Cow, io::Write};

use serde::{Deserialize, Serialize};
use valence_protocol::{Bounded, Encode, anyhow, anyhow::Context};

use crate::{
    color::Color,
    event::{ClickEvent, HoverEvent},
    font::Font,
    scoreboard::ScoreboardValueContent,
};

mod color;
mod event;
mod font;
mod helper;
mod scoreboard;

/// Text data and formatting.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Text<'a> {
    #[serde(flatten)]
    pub content: TextContent<'a>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<Font>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underlined: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obfuscated: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insertion: Option<Cow<'a, str>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_event: Option<Box<ClickEvent<'a>>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover_event: Option<Box<HoverEvent<'a>>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<Self>,
}

const MAX_TEXT_CHARS: usize = 262_144;

impl Encode for Text<'_> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        let s = serde_json::to_string(self).context("serializing text JSON")?;

        Bounded::<_, MAX_TEXT_CHARS>(s).encode(w)
    }
}

/// The text content of a Text object.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextContent<'a> {
    /// Normal text
    Text { text: Cow<'a, str> },
    /// A piece of text that will be translated on the client based on the
    /// client language. If no corresponding translation can be found, the
    /// identifier itself is used as the translated text.
    Translate {
        /// A translation identifier, corresponding to the identifiers found in
        /// loaded language files.
        translate: Cow<'a, str>,
        /// Optional list of text components to be inserted into slots in the
        /// translation text. Ignored if `translate` is not present.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        with: Vec<Text<'a>>,
    },
    /// Displays a score holder's current score in an objective.
    ScoreboardValue { score: ScoreboardValueContent<'a> },
    /// Displays the name of one or more entities found by a [`selector`].
    ///
    /// [`selector`]: https://minecraft.wiki/w/Target_selectors
    EntityNames {
        /// A string containing a [`selector`].
        ///
        /// [`selector`]: https://minecraft.wiki/w/Target_selectors
        selector: Cow<'a, str>,
        /// An optional custom separator used when the selector returns multiple
        /// entities. Defaults to the ", " text with gray color.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        separator: Option<Box<Text<'a>>>,
    },
    /// Displays the name of the button that is currently bound to a certain
    /// configurable control on the client.
    Keybind {
        /// A [`keybind identifier`], to be displayed as the name of the button
        /// that is currently bound to that action.
        ///
        /// [`keybind identifier`]: https://minecraft.wiki/w/Controls#Configurable_controls
        keybind: Cow<'a, str>,
    },
    /// Displays NBT values from block entities.
    BlockNbt {
        block: Cow<'a, str>,
        nbt: Cow<'a, str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interpret: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        separator: Option<Box<Text<'a>>>,
    },
    /// Displays NBT values from entities.
    EntityNbt {
        entity: Cow<'a, str>,
        nbt: Cow<'a, str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interpret: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        separator: Option<Box<Text<'a>>>,
    },
    /// Displays NBT values from command storage.
    StorageNbt {
        storage: Cow<'a, str>,
        nbt: Cow<'a, str>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interpret: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        separator: Option<Box<Text<'a>>>,
    },
}
