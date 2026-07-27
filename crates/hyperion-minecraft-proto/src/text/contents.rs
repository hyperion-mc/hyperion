//! What a component says, before styling.
//!
//! Each variant is one entry in `ComponentSerialization.bootstrap`, and each
//! is recognised by the mandatory field its codec requires. Vanilla's
//! `FuzzyCodec` tries the alternatives in the iteration order of a
//! `HashBiMap`, which is not the registration order and not specified; the
//! mandatory fields are disjoint, so the order only decides which error a
//! malformed value gets. This decodes in the order Mojang registers them.
//!
//! `object` is new and is easy to miss: a component can now be an atlas
//! sprite or a player head rendered inline, with a text `fallback` for
//! clients that cannot draw it.

use std::borrow::Cow;

use crate::{
    Error, Result,
    nbt::{Compound, List, Tag},
    text::{
        Component, field,
        style::{borrowed_string, tag_bool},
    },
};

/// The body of a component.
#[derive(Debug, Clone, PartialEq)]
pub enum Contents<'a> {
    /// Literal text (`PlainTextContents`).
    Text(Cow<'a, str>),
    /// A translation key resolved against the client's language
    /// (`TranslatableContents`).
    Translatable(Translatable<'a>),
    /// The key currently bound to an action, resolved by the client
    /// (`KeybindContents`).
    Keybind(Cow<'a, str>),
    /// A scoreboard value (`ScoreContents`).
    Score(Score<'a>),
    /// The names of the entities an entity selector matches
    /// (`SelectorContents`).
    Selector {
        /// The selector source text, for example `@a[team=red]`.
        selector: Cow<'a, str>,
        /// What to put between names. Defaults to `", "` when absent.
        separator: Option<Box<Component<'a>>>,
    },
    /// NBT read out of the world at render time (`NbtContents`).
    Nbt(NbtContents<'a>),
    /// A sprite drawn inline (`ObjectContents`).
    Object {
        /// What to draw.
        object: ObjectInfo<'a>,
        /// What to show instead when the client will not draw it.
        fallback: Option<Box<Component<'a>>>,
    },
}

/// A translation key and its substitutions.
#[derive(Debug, Clone, PartialEq)]
pub struct Translatable<'a> {
    /// Translation key.
    pub key: Cow<'a, str>,
    /// Text to use when the client has no translation for `key`.
    pub fallback: Option<Cow<'a, str>>,
    /// Values substituted for the `%s` placeholders, in order.
    pub with: Vec<Argument<'a>>,
}

/// One substitution in a translatable component.
///
/// `TranslatableContents.ARG_CODEC` takes either a primitive or a whole
/// component, and `isAllowedPrimitiveArgument` limits the primitive side to a
/// number, a boolean or a string. NBT has no boolean, so a boolean argument
/// arrives as [`Argument::Byte`] and there is no separate variant for it.
#[derive(Debug, Clone, PartialEq)]
pub enum Argument<'a> {
    /// A byte, which is also how a boolean argument arrives.
    Byte(i8),
    /// A short.
    Short(i16),
    /// An int.
    Int(i32),
    /// A long.
    Long(i64),
    /// A float.
    Float(f32),
    /// A double.
    Double(f64),
    /// A string.
    String(Cow<'a, str>),
    /// A nested component.
    Component(Box<Component<'a>>),
}

/// A scoreboard lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Score<'a> {
    /// A player name, an entity selector, or `*` for the reading player.
    pub name: Cow<'a, str>,
    /// Objective to read.
    pub objective: Cow<'a, str>,
}

/// An NBT lookup rendered into the message.
#[derive(Debug, Clone, PartialEq)]
pub struct NbtContents<'a> {
    /// NBT path to read.
    pub path: Cow<'a, str>,
    /// Parse each result as a component rather than showing it as NBT.
    pub interpret: bool,
    /// Show results without the syntax colouring NBT normally gets.
    ///
    /// Mutually exclusive with `interpret`; `NbtContents.MAP_CODEC`'s
    /// `validate` rejects both at once.
    pub plain: bool,
    /// What to put between results.
    pub separator: Option<Box<Component<'a>>>,
    /// Where to read from.
    pub source: DataSource<'a>,
}

/// Where an NBT component reads from (`DataSources`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataSource<'a> {
    /// A block entity, addressed by a coordinate expression.
    Block(Cow<'a, str>),
    /// Entities matched by a selector.
    Entity(Cow<'a, str>),
    /// A command storage namespace.
    Storage(Cow<'a, str>),
}

/// What an object component draws (`ObjectInfos`).
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectInfo<'a> {
    /// A sprite from a texture atlas.
    Atlas {
        /// Atlas id. `minecraft:blocks` when absent (`AtlasIds.BLOCKS`).
        atlas: Cow<'a, str>,
        /// Sprite id within the atlas.
        sprite: Cow<'a, str>,
    },
    /// A player's head.
    Player {
        /// A `ResolvableProfile`: either a bare name or a compound of name,
        /// id and properties. Kept verbatim, since resolving it is the
        /// session server's job and not this crate's.
        player: Tag<'a>,
        /// Whether to draw the hat layer. True when absent.
        hat: bool,
    },
}

/// `AtlasSprite.DEFAULT_ATLAS`.
pub const DEFAULT_ATLAS: &str = "minecraft:blocks";

impl<'a> Contents<'a> {
    pub(super) fn write_into<'t>(&'t self, compound: &mut Compound<'t>) {
        match self {
            Self::Text(text) => {
                compound.insert("text", borrowed_string(text));
            }
            Self::Translatable(translatable) => {
                compound.insert("translate", borrowed_string(&translatable.key));
                compound.insert_optional(
                    "fallback",
                    translatable.fallback.as_deref().map(borrowed_string),
                );
                if !translatable.with.is_empty() {
                    compound.insert(
                        "with",
                        Tag::List(
                            translatable
                                .with
                                .iter()
                                .map(Argument::to_tag)
                                .collect::<List<'t>>(),
                        ),
                    );
                }
            }
            Self::Keybind(name) => {
                compound.insert("keybind", borrowed_string(name));
            }
            Self::Score(score) => {
                let mut inner = Compound::new();
                inner.insert("name", borrowed_string(&score.name));
                inner.insert("objective", borrowed_string(&score.objective));
                compound.insert("score", Tag::Compound(inner));
            }
            Self::Selector {
                selector,
                separator,
            } => {
                compound.insert("selector", borrowed_string(selector));
                compound
                    .insert_optional("separator", separator.as_ref().map(|value| value.to_tag()));
            }
            Self::Nbt(nbt) => {
                compound.insert("nbt", borrowed_string(&nbt.path));
                if nbt.interpret {
                    compound.insert("interpret", tag_bool(true));
                }
                if nbt.plain {
                    compound.insert("plain", tag_bool(true));
                }
                compound.insert_optional(
                    "separator",
                    nbt.separator.as_ref().map(|value| value.to_tag()),
                );
                let (key, value) = match &nbt.source {
                    DataSource::Block(value) => ("block", value),
                    DataSource::Entity(value) => ("entity", value),
                    DataSource::Storage(value) => ("storage", value),
                };
                compound.insert(key, borrowed_string(value));
            }
            Self::Object { object, fallback } => {
                match object {
                    ObjectInfo::Atlas { atlas, sprite } => {
                        if atlas != DEFAULT_ATLAS {
                            compound.insert("atlas", borrowed_string(atlas));
                        }
                        compound.insert("sprite", borrowed_string(sprite));
                    }
                    ObjectInfo::Player { player, hat } => {
                        compound.insert("player", player.clone());
                        if !*hat {
                            compound.insert("hat", tag_bool(false));
                        }
                    }
                }
                compound.insert_optional("fallback", fallback.as_ref().map(|value| value.to_tag()));
            }
        }
    }

    pub(super) fn read_from(compound: &Compound<'a>) -> Result<Self> {
        if let Some(text) = field::optional_string(compound, "text")? {
            return Ok(Self::Text(text));
        }
        if let Some(key) = field::optional_string(compound, "translate")? {
            return Ok(Self::Translatable(Translatable {
                key,
                fallback: field::optional_string(compound, "fallback")?,
                with: read_arguments(compound)?,
            }));
        }
        if let Some(name) = field::optional_string(compound, "keybind")? {
            return Ok(Self::Keybind(name));
        }
        if compound.get("score").is_some() {
            let inner = field::compound(compound, "score")?;
            return Ok(Self::Score(Score {
                name: field::string(inner, "name")?,
                objective: field::string(inner, "objective")?,
            }));
        }
        if let Some(selector) = field::optional_string(compound, "selector")? {
            return Ok(Self::Selector {
                selector,
                separator: read_component(compound, "separator")?,
            });
        }
        if let Some(path) = field::optional_string(compound, "nbt")? {
            return Ok(Self::Nbt(NbtContents {
                path,
                interpret: field::bool_or(compound, "interpret", false)?,
                plain: field::bool_or(compound, "plain", false)?,
                separator: read_component(compound, "separator")?,
                source: DataSource::read_from(compound)?,
            }));
        }
        if let Some(object) = ObjectInfo::read_from(compound)? {
            return Ok(Self::Object {
                object,
                fallback: read_component(compound, "fallback")?,
            });
        }
        Err(Error::NoMatchingCodec("component contents"))
    }
}

impl<'a> DataSource<'a> {
    fn read_from(compound: &Compound<'a>) -> Result<Self> {
        if let Some(value) = field::optional_string(compound, "entity")? {
            return Ok(Self::Entity(value));
        }
        if let Some(value) = field::optional_string(compound, "block")? {
            return Ok(Self::Block(value));
        }
        if let Some(value) = field::optional_string(compound, "storage")? {
            return Ok(Self::Storage(value));
        }
        Err(Error::NoMatchingCodec("nbt data source"))
    }
}

impl<'a> ObjectInfo<'a> {
    fn read_from(compound: &Compound<'a>) -> Result<Option<Self>> {
        if let Some(sprite) = field::optional_string(compound, "sprite")? {
            return Ok(Some(Self::Atlas {
                atlas: field::optional_string(compound, "atlas")?
                    .unwrap_or(Cow::Borrowed(DEFAULT_ATLAS)),
                sprite,
            }));
        }
        if let Some(player) = compound.get("player") {
            return Ok(Some(Self::Player {
                player: player.clone(),
                hat: field::bool_or(compound, "hat", true)?,
            }));
        }
        Ok(None)
    }
}

impl<'a> Argument<'a> {
    fn to_tag(&self) -> Tag<'_> {
        match self {
            Self::Byte(value) => Tag::Byte(*value),
            Self::Short(value) => Tag::Short(*value),
            Self::Int(value) => Tag::Int(*value),
            Self::Long(value) => Tag::Long(*value),
            Self::Float(value) => Tag::Float(*value),
            Self::Double(value) => Tag::Double(*value),
            Self::String(value) => borrowed_string(value),
            Self::Component(component) => component.to_tag(),
        }
    }

    fn from_tag(tag: &Tag<'a>) -> Result<Self> {
        Ok(match tag {
            Tag::Byte(value) => Self::Byte(*value),
            Tag::Short(value) => Self::Short(*value),
            Tag::Int(value) => Self::Int(*value),
            Tag::Long(value) => Self::Long(*value),
            Tag::Float(value) => Self::Float(*value),
            Tag::Double(value) => Self::Double(*value),
            // A string is a primitive argument, never a collapsed component:
            // ARG_CODEC tries the primitive codec first.
            Tag::String(value) => Self::String(value.clone()),
            other => Self::Component(Box::new(Component::from_tag(other)?)),
        })
    }
}

fn read_arguments<'a>(compound: &Compound<'a>) -> Result<Vec<Argument<'a>>> {
    let Some(tag) = compound.get("with") else {
        return Ok(Vec::new());
    };
    let list = tag.as_list().ok_or_else(|| Error::WrongTagType {
        field: "with",
        expected: "TAG_List",
        found: tag.id(),
    })?;
    list.as_slice().iter().map(Argument::from_tag).collect()
}

fn read_component<'a>(
    compound: &Compound<'a>,
    field: &'static str,
) -> Result<Option<Box<Component<'a>>>> {
    compound
        .get(field)
        .map(|tag| Component::from_tag(tag).map(Box::new))
        .transpose()
}
