//! Component styling.
//!
//! Every field is optional and every absent field inherits from the parent,
//! which is why the booleans are `Option<bool>` rather than `bool`: `Style`
//! distinguishes "not set" from "set to false", and `applyTo` relies on it.
//!
//! The field names are `snake_case`. `clickEvent` and `hoverEvent` were
//! renamed to `click_event` and `hover_event` when the format moved to NBT;
//! documentation written for the JSON era still shows the old spelling, and
//! `Style.Serializer.MAP_CODEC` is where the current one is fixed.

use std::borrow::Cow;

use crate::{
    Error, Result,
    nbt::{Compound, Tag},
    text::{
        event::{ClickEvent, HoverEvent},
        field,
    },
};

/// Formatting applied to a component and inherited by its children.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style<'a> {
    /// Text colour.
    pub color: Option<TextColor>,
    /// Drop-shadow colour as packed ARGB, where zero means no shadow.
    pub shadow_color: Option<i32>,
    /// Bold.
    pub bold: Option<bool>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underlined.
    pub underlined: Option<bool>,
    /// Struck through.
    pub strikethrough: Option<bool>,
    /// Obfuscated, the scrambling "magic" format.
    pub obfuscated: Option<bool>,
    /// What clicking the text does.
    pub click_event: Option<ClickEvent<'a>>,
    /// What hovering over the text shows.
    pub hover_event: Option<HoverEvent<'a>>,
    /// Text inserted into the chat box when the text is shift-clicked.
    pub insertion: Option<Cow<'a, str>>,
    /// Font resource id.
    ///
    /// Only `FontDescription.Resource` has a serialisation; the sprite
    /// variants exist so that an object component can point the renderer at a
    /// glyph, and `FontDescription.CODEC` refuses to encode them.
    pub font: Option<Cow<'a, str>>,
}

impl<'a> Style<'a> {
    /// A style that sets nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color: None,
            shadow_color: None,
            bold: None,
            italic: None,
            underlined: None,
            strikethrough: None,
            obfuscated: None,
            click_event: None,
            hover_event: None,
            insertion: None,
            font: None,
        }
    }

    /// True when no field is set, which is `Style.EMPTY`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.shadow_color.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underlined.is_none()
            && self.strikethrough.is_none()
            && self.obfuscated.is_none()
            && self.click_event.is_none()
            && self.hover_event.is_none()
            && self.insertion.is_none()
            && self.font.is_none()
    }

    pub(super) fn write_into<'t>(&'t self, compound: &mut Compound<'t>) {
        compound.insert_optional("color", self.color.map(TextColor::to_tag));
        compound.insert_optional("shadow_color", self.shadow_color.map(Tag::Int));
        compound.insert_optional("bold", self.bold.map(tag_bool));
        compound.insert_optional("italic", self.italic.map(tag_bool));
        compound.insert_optional("underlined", self.underlined.map(tag_bool));
        compound.insert_optional("strikethrough", self.strikethrough.map(tag_bool));
        compound.insert_optional("obfuscated", self.obfuscated.map(tag_bool));
        compound.insert_optional(
            "click_event",
            self.click_event.as_ref().map(ClickEvent::to_tag),
        );
        compound.insert_optional(
            "hover_event",
            self.hover_event.as_ref().map(HoverEvent::to_tag),
        );
        compound.insert_optional("insertion", self.insertion.as_deref().map(borrowed_string));
        compound.insert_optional("font", self.font.as_deref().map(borrowed_string));
    }

    pub(super) fn read_from(compound: &Compound<'a>) -> Result<Self> {
        Ok(Self {
            color: field::optional_string(compound, "color")?
                .map(|name| TextColor::parse(&name))
                .transpose()?,
            shadow_color: field::optional_int(compound, "shadow_color")?,
            bold: field::optional_bool(compound, "bold")?,
            italic: field::optional_bool(compound, "italic")?,
            underlined: field::optional_bool(compound, "underlined")?,
            strikethrough: field::optional_bool(compound, "strikethrough")?,
            obfuscated: field::optional_bool(compound, "obfuscated")?,
            click_event: compound
                .get("click_event")
                .map(ClickEvent::from_tag)
                .transpose()?,
            hover_event: compound
                .get("hover_event")
                .map(HoverEvent::from_tag)
                .transpose()?,
            insertion: field::optional_string(compound, "insertion")?,
            font: field::optional_string(compound, "font")?,
        })
    }
}

/// A text colour, which travels as either a name or a `#RRGGBB` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextColor {
    /// One of the sixteen legacy colours, written as its name.
    Named(NamedColor),
    /// A 24-bit colour, written as `#RRGGBB` with uppercase hex digits.
    Rgb(u32),
}

impl TextColor {
    /// Parse the string form (`TextColor.parseColor`).
    ///
    /// # Errors
    /// Returns [`Error::UnknownVariant`] for an unknown name or a hex value
    /// outside 24 bits.
    pub fn parse(value: &str) -> Result<Self> {
        if let Some(digits) = value.strip_prefix('#') {
            let parsed = u32::from_str_radix(digits, 16)
                .ok()
                .filter(|rgb| *rgb <= 0x00FF_FFFF);
            return parsed.map(Self::Rgb).ok_or_else(|| Error::UnknownVariant {
                name: "TextColor",
                value: value.to_owned(),
            });
        }
        NamedColor::parse(value)
            .map(Self::Named)
            .ok_or_else(|| Error::UnknownVariant {
                name: "TextColor",
                value: value.to_owned(),
            })
    }

    /// The 24-bit value this colour renders as.
    #[must_use]
    pub const fn rgb(self) -> u32 {
        match self {
            Self::Named(named) => named.rgb(),
            Self::Rgb(value) => value,
        }
    }

    fn to_tag<'a>(self) -> Tag<'a> {
        match self {
            Self::Named(named) => Tag::String(Cow::Borrowed(named.as_str())),
            Self::Rgb(value) => Tag::String(Cow::Owned(format!("#{value:06X}"))),
        }
    }
}

/// The sixteen colours `ChatFormatting` predates the hex form with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedColor {
    /// `black`.
    Black,
    /// `dark_blue`.
    DarkBlue,
    /// `dark_green`.
    DarkGreen,
    /// `dark_aqua`.
    DarkAqua,
    /// `dark_red`.
    DarkRed,
    /// `dark_purple`.
    DarkPurple,
    /// `gold`.
    Gold,
    /// `gray`.
    Gray,
    /// `dark_gray`.
    DarkGray,
    /// `blue`.
    Blue,
    /// `green`.
    Green,
    /// `aqua`.
    Aqua,
    /// `red`.
    Red,
    /// `light_purple`.
    LightPurple,
    /// `yellow`.
    Yellow,
    /// `white`.
    White,
}

impl NamedColor {
    /// Every colour, in the order `TextColor` declares them.
    pub const ALL: [Self; 16] = [
        Self::Black,
        Self::DarkBlue,
        Self::DarkGreen,
        Self::DarkAqua,
        Self::DarkRed,
        Self::DarkPurple,
        Self::Gold,
        Self::Gray,
        Self::DarkGray,
        Self::Blue,
        Self::Green,
        Self::Aqua,
        Self::Red,
        Self::LightPurple,
        Self::Yellow,
        Self::White,
    ];

    /// The serialised name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::DarkBlue => "dark_blue",
            Self::DarkGreen => "dark_green",
            Self::DarkAqua => "dark_aqua",
            Self::DarkRed => "dark_red",
            Self::DarkPurple => "dark_purple",
            Self::Gold => "gold",
            Self::Gray => "gray",
            Self::DarkGray => "dark_gray",
            Self::Blue => "blue",
            Self::Green => "green",
            Self::Aqua => "aqua",
            Self::Red => "red",
            Self::LightPurple => "light_purple",
            Self::Yellow => "yellow",
            Self::White => "white",
        }
    }

    /// The 24-bit value this colour renders as, as `TextColor.named` registers it.
    #[must_use]
    pub const fn rgb(self) -> u32 {
        match self {
            Self::Black => 0x0000_0000,
            Self::DarkBlue => 0x0000_00AA,
            Self::DarkGreen => 0x0000_AA00,
            Self::DarkAqua => 0x0000_AAAA,
            Self::DarkRed => 0x00AA_0000,
            Self::DarkPurple => 0x00AA_00AA,
            Self::Gold => 0x00FF_AA00,
            Self::Gray => 0x00AA_AAAA,
            Self::DarkGray => 0x0055_5555,
            Self::Blue => 0x0055_55FF,
            Self::Green => 0x0055_FF55,
            Self::Aqua => 0x0055_FFFF,
            Self::Red => 0x00FF_5555,
            Self::LightPurple => 0x00FF_55FF,
            Self::Yellow => 0x00FF_FF55,
            Self::White => 0x00FF_FFFF,
        }
    }

    /// The colour with this name, if there is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.as_str() == name)
    }
}

/// NBT has no boolean type; `NbtOps.createBoolean` writes a byte.
pub(super) const fn tag_bool<'a>(value: bool) -> Tag<'a> {
    Tag::Byte(if value { 1 } else { 0 })
}

pub(super) const fn borrowed_string(value: &str) -> Tag<'_> {
    Tag::String(Cow::Borrowed(value))
}
