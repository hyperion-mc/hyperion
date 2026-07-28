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

    /// The same style with `color` set.
    #[must_use]
    pub fn color(mut self, color: impl Into<TextColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// What this style says about `decoration`: set on, set off, or unset and
    /// therefore inherited.
    #[must_use]
    pub const fn decoration(&self, decoration: Decoration) -> Option<bool> {
        match decoration {
            Decoration::Bold => self.bold,
            Decoration::Italic => self.italic,
            Decoration::Underlined => self.underlined,
            Decoration::Strikethrough => self.strikethrough,
            Decoration::Obfuscated => self.obfuscated,
        }
    }

    /// The same style with `decoration` set to `value`, where `None` means
    /// inherit.
    #[must_use]
    pub const fn set_decoration(mut self, decoration: Decoration, value: Option<bool>) -> Self {
        let slot = match decoration {
            Decoration::Bold => &mut self.bold,
            Decoration::Italic => &mut self.italic,
            Decoration::Underlined => &mut self.underlined,
            Decoration::Strikethrough => &mut self.strikethrough,
            Decoration::Obfuscated => &mut self.obfuscated,
        };
        *slot = value;
        self
    }

    /// The same style with `decoration` turned on.
    #[must_use]
    pub const fn with(self, decoration: Decoration) -> Self {
        self.set_decoration(decoration, Some(true))
    }

    /// The same style with `decoration` turned off.
    ///
    /// Distinct from [`inherit`](Self::inherit): off overrides a parent that
    /// turned it on, which is the only way to un-italicise an item name.
    #[must_use]
    pub const fn without(self, decoration: Decoration) -> Self {
        self.set_decoration(decoration, Some(false))
    }

    /// The same style with `decoration` left to the parent.
    #[must_use]
    pub const fn inherit(self, decoration: Decoration) -> Self {
        self.set_decoration(decoration, None)
    }

    /// Bold. Shorthand for [`with`](Self::with).
    #[must_use]
    pub const fn bold(self) -> Self {
        self.with(Decoration::Bold)
    }

    /// Italic. Shorthand for [`with`](Self::with).
    #[must_use]
    pub const fn italic(self) -> Self {
        self.with(Decoration::Italic)
    }

    /// Underlined. Shorthand for [`with`](Self::with).
    #[must_use]
    pub const fn underlined(self) -> Self {
        self.with(Decoration::Underlined)
    }

    /// Struck through. Shorthand for [`with`](Self::with).
    #[must_use]
    pub const fn strikethrough(self) -> Self {
        self.with(Decoration::Strikethrough)
    }

    /// Obfuscated. Shorthand for [`with`](Self::with).
    #[must_use]
    pub const fn obfuscated(self) -> Self {
        self.with(Decoration::Obfuscated)
    }

    /// The same style with a drop shadow of this packed ARGB colour. Zero is
    /// no shadow, which is not the same as leaving it unset.
    #[must_use]
    pub const fn shadow(mut self, argb: i32) -> Self {
        self.shadow_color = Some(argb);
        self
    }

    /// The same style drawn in `font`.
    #[must_use]
    pub fn font(mut self, font: impl Into<Cow<'a, str>>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// The same style with text to put in the chat box on shift-click.
    #[must_use]
    pub fn insertion(mut self, text: impl Into<Cow<'a, str>>) -> Self {
        self.insertion = Some(text.into());
        self
    }

    /// The same style with a click action.
    #[must_use]
    pub fn on_click(mut self, event: ClickEvent<'a>) -> Self {
        self.click_event = Some(event);
        self
    }

    /// The same style with a hover tooltip.
    #[must_use]
    pub fn on_hover(mut self, event: HoverEvent<'a>) -> Self {
        self.hover_event = Some(event);
        self
    }

    /// This style resolved against the one it is nested inside
    /// (`Style.applyTo`).
    ///
    /// Every field this style sets wins; every field it leaves unset is taken
    /// from `parent`. This is the rule the client applies when it walks a
    /// component tree, so calling it is how the server can know what a run
    /// will actually look like.
    #[must_use]
    pub fn inheriting(self, parent: &Self) -> Self {
        Self {
            color: self.color.or(parent.color),
            shadow_color: self.shadow_color.or(parent.shadow_color),
            bold: self.bold.or(parent.bold),
            italic: self.italic.or(parent.italic),
            underlined: self.underlined.or(parent.underlined),
            strikethrough: self.strikethrough.or(parent.strikethrough),
            obfuscated: self.obfuscated.or(parent.obfuscated),
            click_event: self.click_event.or_else(|| parent.click_event.clone()),
            hover_event: self.hover_event.or_else(|| parent.hover_event.clone()),
            insertion: self.insertion.or_else(|| parent.insertion.clone()),
            font: self.font.or_else(|| parent.font.clone()),
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

/// A 24-bit colour, which is the whole range `#RRGGBB` can spell.
///
/// The field is private because the wire format has no notation for anything
/// wider. `TextColor::parse` has always rejected a hex string above
/// `#FFFFFF`, but `TextColor::Rgb(0x0100_0000)` was a value the same type
/// could hold and `to_tag` would have written out as seven hex digits, which
/// no client can read back. Going through [`Rgb24::new`] or
/// [`Rgb24::from_u24`] is what removes that case from the type rather than
/// catching it at send time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rgb24(u32);

impl Rgb24 {
    /// The colour with these channel values.
    ///
    /// Total, because three bytes are a 24-bit colour by construction. This is
    /// the constructor to reach for; [`from_u24`](Self::from_u24) exists for
    /// data that already arrives packed.
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self(u32::from_be_bytes([0, red, green, blue]))
    }

    /// The colour packed as `0x00RRGGBB`, or `None` when a bit above the low
    /// twenty-four is set.
    #[must_use]
    pub const fn from_u24(packed: u32) -> Option<Self> {
        if packed > 0x00FF_FFFF {
            None
        } else {
            Some(Self(packed))
        }
    }

    /// The packed `0x00RRGGBB` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Red, green and blue.
    #[must_use]
    pub const fn channels(self) -> [u8; 3] {
        let [_, red, green, blue] = self.0.to_be_bytes();
        [red, green, blue]
    }
}

/// A text colour, which travels as either a name or a `#RRGGBB` string.
///
/// Both forms are always available: `TextColor` gained the hex spelling in
/// 1.16 and this crate speaks 1.21, so there is no protocol version reachable
/// from here where [`Rgb24`] has to fall back to a named approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextColor {
    /// One of the sixteen legacy colours, written as its name.
    Named(NamedColor),
    /// A 24-bit colour, written as `#RRGGBB` with uppercase hex digits.
    Rgb(Rgb24),
}

impl From<NamedColor> for TextColor {
    fn from(named: NamedColor) -> Self {
        Self::Named(named)
    }
}

impl From<Rgb24> for TextColor {
    fn from(rgb: Rgb24) -> Self {
        Self::Rgb(rgb)
    }
}

impl TextColor {
    /// Parse the string form (`TextColor.parseColor`).
    ///
    /// # Errors
    /// Returns [`Error::UnknownVariant`] for an unknown name or a hex value
    /// outside 24 bits.
    pub fn parse(value: &str) -> Result<Self> {
        let unknown = || Error::UnknownVariant {
            name: "TextColor",
            value: value.to_owned(),
        };
        if let Some(digits) = value.strip_prefix('#') {
            return u32::from_str_radix(digits, 16)
                .ok()
                .and_then(Rgb24::from_u24)
                .map(Self::Rgb)
                .ok_or_else(unknown);
        }
        NamedColor::parse(value)
            .map(Self::Named)
            .ok_or_else(unknown)
    }

    /// The 24-bit value this colour renders as.
    #[must_use]
    pub const fn rgb(self) -> Rgb24 {
        match self {
            Self::Named(named) => named.rgb(),
            Self::Rgb(value) => value,
        }
    }

    fn to_tag<'a>(self) -> Tag<'a> {
        match self {
            Self::Named(named) => Tag::String(Cow::Borrowed(named.as_str())),
            Self::Rgb(value) => Tag::String(Cow::Owned(format!("#{:06X}", value.get()))),
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
    pub const fn rgb(self) -> Rgb24 {
        match self {
            Self::Black => Rgb24::new(0x00, 0x00, 0x00),
            Self::DarkBlue => Rgb24::new(0x00, 0x00, 0xAA),
            Self::DarkGreen => Rgb24::new(0x00, 0xAA, 0x00),
            Self::DarkAqua => Rgb24::new(0x00, 0xAA, 0xAA),
            Self::DarkRed => Rgb24::new(0xAA, 0x00, 0x00),
            Self::DarkPurple => Rgb24::new(0xAA, 0x00, 0xAA),
            Self::Gold => Rgb24::new(0xFF, 0xAA, 0x00),
            Self::Gray => Rgb24::new(0xAA, 0xAA, 0xAA),
            Self::DarkGray => Rgb24::new(0x55, 0x55, 0x55),
            Self::Blue => Rgb24::new(0x55, 0x55, 0xFF),
            Self::Green => Rgb24::new(0x55, 0xFF, 0x55),
            Self::Aqua => Rgb24::new(0x55, 0xFF, 0xFF),
            Self::Red => Rgb24::new(0xFF, 0x55, 0x55),
            Self::LightPurple => Rgb24::new(0xFF, 0x55, 0xFF),
            Self::Yellow => Rgb24::new(0xFF, 0xFF, 0x55),
            Self::White => Rgb24::new(0xFF, 0xFF, 0xFF),
        }
    }

    /// The colour with this name, if there is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.as_str() == name)
    }
}

/// One of the five boolean formats a style carries, which is `ChatFormatting`
/// minus its colours and its reset.
///
/// Named as a type rather than left as five fields so that a caller can write
/// one function over all of them, and so that [`Decoration::ALL`] gives a test
/// something to enumerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Decoration {
    /// `bold`.
    Bold,
    /// `italic`.
    Italic,
    /// `underlined`.
    Underlined,
    /// `strikethrough`.
    Strikethrough,
    /// `obfuscated`, the scrambling "magic" format.
    Obfuscated,
}

impl Decoration {
    /// Every decoration, in the order [`Style`] declares the fields.
    pub const ALL: [Self; 5] = [
        Self::Bold,
        Self::Italic,
        Self::Underlined,
        Self::Strikethrough,
        Self::Obfuscated,
    ];

    /// The field name this decoration is written under.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Underlined => "underlined",
            Self::Strikethrough => "strikethrough",
            Self::Obfuscated => "obfuscated",
        }
    }
}

/// NBT has no boolean type; `NbtOps.createBoolean` writes a byte.
pub(super) const fn tag_bool<'a>(value: bool) -> Tag<'a> {
    Tag::Byte(if value { 1 } else { 0 })
}

pub(super) const fn borrowed_string(value: &str) -> Tag<'_> {
    Tag::String(Cow::Borrowed(value))
}
