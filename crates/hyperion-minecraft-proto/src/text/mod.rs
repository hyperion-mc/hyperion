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
    style::{Decoration, NamedColor, Rgb24, Style, TextColor},
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

    /// The same component with every one of `children` appended.
    #[must_use]
    pub fn extend(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.extra.extend(children);
        self
    }

    /// The same component drawn in `color`.
    #[must_use]
    pub fn color(mut self, color: impl Into<TextColor>) -> Self {
        self.style = self.style.color(color);
        self
    }

    /// The same component with `decoration` turned on.
    #[must_use]
    pub fn with(mut self, decoration: Decoration) -> Self {
        self.style = self.style.with(decoration);
        self
    }

    /// The same component with `decoration` turned off, overriding a parent
    /// that turned it on.
    #[must_use]
    pub fn without(mut self, decoration: Decoration) -> Self {
        self.style = self.style.without(decoration);
        self
    }

    /// Bold.
    #[must_use]
    pub fn bold(self) -> Self {
        self.with(Decoration::Bold)
    }

    /// Italic.
    #[must_use]
    pub fn italic(self) -> Self {
        self.with(Decoration::Italic)
    }

    /// Underlined.
    #[must_use]
    pub fn underlined(self) -> Self {
        self.with(Decoration::Underlined)
    }

    /// Struck through.
    #[must_use]
    pub fn strikethrough(self) -> Self {
        self.with(Decoration::Strikethrough)
    }

    /// Obfuscated.
    #[must_use]
    pub fn obfuscated(self) -> Self {
        self.with(Decoration::Obfuscated)
    }

    /// The same component drawn in `font`.
    #[must_use]
    pub fn font(mut self, font: impl Into<Cow<'a, str>>) -> Self {
        self.style = self.style.font(font);
        self
    }

    /// The same component with a click action.
    #[must_use]
    pub fn on_click(mut self, event: ClickEvent<'a>) -> Self {
        self.style = self.style.on_click(event);
        self
    }

    /// The same component with a hover tooltip.
    #[must_use]
    pub fn on_hover(mut self, event: HoverEvent<'a>) -> Self {
        self.style = self.style.on_hover(event);
        self
    }

    /// The literal runs a client draws this component as, with style
    /// inheritance already resolved.
    ///
    /// Only [`Contents::Text`] contributes a run. A translation key, a
    /// keybind, a score or an NBT path is text the *client* produces, and the
    /// server cannot know what it will say; those contribute nothing rather
    /// than a guess. So `runs` answers "what has this server spelled out, and
    /// in what colour", which is exactly the question a layout calculation and
    /// a rendering test each want, and it does not pretend to answer "what
    /// will appear on screen".
    #[must_use]
    pub fn runs(&self) -> Vec<Run<'_>> {
        let mut runs = Vec::new();
        self.collect_runs(&Style::new(), &mut runs);
        runs
    }

    fn collect_runs<'s>(&'s self, inherited: &Style<'s>, runs: &mut Vec<Run<'s>>) {
        let style = self.style.clone().inheriting(inherited);
        if let Contents::Text(text) = &self.contents
            && !text.is_empty()
        {
            runs.push(Run {
                text: Cow::Borrowed(text.as_ref()),
                style: style.clone(),
            });
        }
        for child in &self.extra {
            child.collect_runs(&style, runs);
        }
    }

    /// The literal text of every run, concatenated (`Component.getString`).
    ///
    /// The width a row takes on screen is measured from this and not from the
    /// component, because style costs no columns.
    #[must_use]
    pub fn plain(&self) -> String {
        self.runs().into_iter().map(|run| run.text).collect()
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

/// One stretch of literal text and the style a client draws it in.
///
/// Produced by [`Component::runs`], where the style is already resolved
/// against every ancestor, so a `Run` needs no context to be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Run<'a> {
    /// The literal text.
    pub text: Cow<'a, str>,
    /// The style it is drawn in, with inheritance applied.
    pub style: Style<'a>,
}

impl Run<'_> {
    /// The colour this run is drawn in, or `None` for the client's default.
    #[must_use]
    pub const fn color(&self) -> Option<TextColor> {
        self.style.color
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
