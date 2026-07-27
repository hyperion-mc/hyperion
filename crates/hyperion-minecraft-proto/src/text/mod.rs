//! Text components.
//!
//! Since 1.20.3 a component on the wire is NBT, not JSON:
//! `ComponentSerialization.STREAM_CODEC` is
//! `ByteBufCodecs.fromCodecWithRegistries(CODEC)`, which runs the DFU codec
//! against `NbtOps.INSTANCE` and hands the resulting tag to
//! `ByteBufCodecs.tagCodec`. So [`Component`] encodes through [`crate::nbt`]
//! and carries no JSON anywhere. The two places JSON survives are the status
//! response and the login disconnect, both of which run before any registry
//! has been sent; those stay strings in [`crate::packets`].
//!
//! Three shapes decode to a component, because the top-level codec is
//! `Codec.either(Codec.either(STRING, list), fullCodec)`:
//!
//! - a bare string is a literal with no style,
//! - a non-empty list is its first element with the rest appended as children,
//! - a compound is contents, `extra` and style flattened together.
//!
//! Encoding only ever produces the first and the third. `tryCollapseToString`
//! decides: a literal with no style and no children goes out as a bare string,
//! and everything else as a compound. A vanilla server encoding
//! `Component.literal("hi")` writes five bytes, `08 00 02 68 69`, and not a
//! compound with a `text` field.
//!
//! There is no `type` field. The compound form is dispatched by which
//! mandatory field is present, `text` or `translate` or `keybind` and so on:
//! `createLegacyComponentMatcher` builds a `StrictEither` over a `FuzzyCodec`,
//! and `StrictEither.encode` always delegates to the fuzzy side. A `type`
//! field is accepted on decode and never written.

pub mod contents;
pub mod event;
pub mod style;

mod field;

use std::borrow::Cow;

pub use crate::text::{
    contents::{Argument, Contents, DataSource, NbtContents, ObjectInfo, Score, Translatable},
    event::{ClickEvent, HoverEvent},
    style::{NamedColor, Style, TextColor},
};
use crate::{
    Decode, Encode, Error, Reader, Result, Writer,
    nbt::{Compound, List, Tag},
};

/// A piece of formatted text.
///
/// A component is its contents, the children that follow it, and the style
/// both it and those children inherit.
#[derive(Debug, Clone, PartialEq)]
pub struct Component<'a> {
    /// What this component says.
    pub contents: Contents<'a>,
    /// Children appended after the contents, inheriting this style.
    pub extra: Vec<Self>,
    /// Formatting.
    pub style: Style<'a>,
}

impl<'a> Component<'a> {
    /// A literal string with no style and no children.
    pub fn text(text: impl Into<Cow<'a, str>>) -> Self {
        Self::from_contents(Contents::Text(text.into()))
    }

    /// A translation key with no arguments.
    pub fn translatable(key: impl Into<Cow<'a, str>>) -> Self {
        Self::from_contents(Contents::Translatable(Translatable {
            key: key.into(),
            fallback: None,
            with: Vec::new(),
        }))
    }

    /// A component with the given contents, no style and no children.
    #[must_use]
    pub const fn from_contents(contents: Contents<'a>) -> Self {
        Self {
            contents,
            extra: Vec::new(),
            style: Style::new(),
        }
    }

    /// The same component with `style` applied.
    #[must_use]
    pub fn with_style(mut self, style: Style<'a>) -> Self {
        self.style = style;
        self
    }

    /// The same component with `child` appended.
    #[must_use]
    pub fn append(mut self, child: Self) -> Self {
        self.extra.push(child);
        self
    }

    /// The literal text this component collapses to, if it collapses at all
    /// (`Component.tryCollapseToString`).
    #[must_use]
    pub fn try_collapse_to_str(&self) -> Option<&str> {
        match &self.contents {
            Contents::Text(text) if self.extra.is_empty() && self.style.is_empty() => Some(text),
            _ => None,
        }
    }

    /// The NBT this component encodes to.
    #[must_use]
    pub fn to_tag(&self) -> Tag<'_> {
        if let Some(text) = self.try_collapse_to_str() {
            return Tag::String(Cow::Borrowed(text));
        }
        let mut compound = Compound::new();
        self.contents.write_into(&mut compound);
        if !self.extra.is_empty() {
            compound.insert(
                "extra",
                Tag::List(self.extra.iter().map(Self::to_tag).collect::<List<'_>>()),
            );
        }
        self.style.write_into(&mut compound);
        Tag::Compound(compound)
    }

    /// Read a component out of decoded NBT.
    ///
    /// # Errors
    /// Returns an error when no content codec matches, when a field has the
    /// wrong type, or when a discriminant is unknown.
    pub fn from_tag(tag: &Tag<'a>) -> Result<Self> {
        match tag {
            Tag::String(text) => Ok(Self::text(text.clone())),
            Tag::List(list) => {
                // ExtraCodecs.nonEmptyList, then ComponentSerialization
                // .createFromList: the head absorbs the tail as children.
                let (head, tail) = list
                    .as_slice()
                    .split_first()
                    .ok_or(Error::MissingField("component list"))?;
                let mut component = Self::from_tag(head)?;
                for element in tail {
                    component.extra.push(Self::from_tag(element)?);
                }
                Ok(component)
            }
            Tag::Compound(compound) => Ok(Self {
                contents: Contents::read_from(compound)?,
                extra: read_extra(compound)?,
                style: Style::read_from(compound)?,
            }),
            other => Err(Error::WrongTagType {
                field: "component",
                expected: "TAG_String, TAG_List or TAG_Compound",
                found: other.id(),
            }),
        }
    }
}

impl Encode for Component<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.to_tag().encode(writer)
    }
}

impl<'a> Decode<'a> for Component<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Self::from_tag(&Tag::decode(reader)?)
    }
}

fn read_extra<'a>(compound: &Compound<'a>) -> Result<Vec<Component<'a>>> {
    let Some(tag) = compound.get("extra") else {
        return Ok(Vec::new());
    };
    let list = tag.as_list().ok_or_else(|| Error::WrongTagType {
        field: "extra",
        expected: "TAG_List",
        found: tag.id(),
    })?;
    if list.is_empty() {
        // ExtraCodecs.nonEmptyList rejects a present-but-empty list rather
        // than treating it as the default.
        return Err(Error::MissingField("extra"));
    }
    list.as_slice().iter().map(Component::from_tag).collect()
}
