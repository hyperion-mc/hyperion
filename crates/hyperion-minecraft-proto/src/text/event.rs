//! Click and hover events.
//!
//! Both are `Action.CODEC.dispatch("action", ...)`, so each travels as a
//! compound with an `action` string naming the variant and that variant's own
//! fields alongside it.
//!
//! `ClickEvent.Action.OPEN_FILE` has no variant here. It exists in the enum
//! but `Action.CODEC` is `UNSAFE_CODEC.validate(filterForSerialization)`, and
//! `filterForSerialization` rejects any action whose `allowFromServer` is
//! false. `validate` runs on encode and decode alike, so `open_file` cannot
//! legally appear on the wire in either direction; it is reachable only from a
//! local resource pack.

use std::borrow::Cow;

use crate::{
    Error, Result,
    nbt::{Compound, Tag},
    text::{Component, field, style::borrowed_string},
};

/// What clicking a component does.
#[derive(Debug, Clone, PartialEq)]
pub enum ClickEvent<'a> {
    /// Open a URL. The client validates the scheme before following it.
    OpenUrl(Cow<'a, str>),
    /// Run a command as the player.
    RunCommand(Cow<'a, str>),
    /// Put a command in the chat box without sending it.
    SuggestCommand(Cow<'a, str>),
    /// Open a dialog.
    ///
    /// `Dialog.CODEC` resolves a `Holder<Dialog>`, so the value is either a
    /// registry id or a whole inline dialog whose shape depends on the
    /// registries the connection was sent. It is kept verbatim rather than
    /// modelled, since nothing here can resolve it.
    ShowDialog(Tag<'a>),
    /// Turn to a page of the open book. One-based; zero is rejected.
    ChangePage(i32),
    /// Copy a string to the system clipboard.
    CopyToClipboard(Cow<'a, str>),
    /// A custom event delivered to server-side listeners.
    Custom {
        /// Identifier the listener registered under.
        id: Cow<'a, str>,
        /// Arbitrary payload.
        payload: Option<Tag<'a>>,
    },
}

impl<'a> ClickEvent<'a> {
    /// The `action` discriminant.
    #[must_use]
    pub const fn action(&self) -> &'static str {
        match self {
            Self::OpenUrl(_) => "open_url",
            Self::RunCommand(_) => "run_command",
            Self::SuggestCommand(_) => "suggest_command",
            Self::ShowDialog(_) => "show_dialog",
            Self::ChangePage(_) => "change_page",
            Self::CopyToClipboard(_) => "copy_to_clipboard",
            Self::Custom { .. } => "custom",
        }
    }

    pub(super) fn to_tag(&self) -> Tag<'_> {
        let mut compound = Compound::new();
        compound.insert("action", borrowed_string(self.action()));
        match self {
            Self::OpenUrl(url) => {
                compound.insert("url", borrowed_string(url));
            }
            Self::RunCommand(command) | Self::SuggestCommand(command) => {
                compound.insert("command", borrowed_string(command));
            }
            Self::ShowDialog(dialog) => {
                compound.insert("dialog", dialog.clone());
            }
            Self::ChangePage(page) => {
                compound.insert("page", Tag::Int(*page));
            }
            Self::CopyToClipboard(value) => {
                compound.insert("value", borrowed_string(value));
            }
            Self::Custom { id, payload } => {
                compound.insert("id", borrowed_string(id));
                compound.insert_optional("payload", payload.clone());
            }
        }
        Tag::Compound(compound)
    }

    pub(super) fn from_tag(tag: &Tag<'a>) -> Result<Self> {
        let compound = as_compound(tag, "click_event")?;
        let action = field::string(compound, "action")?;
        Ok(match action.as_ref() {
            "open_url" => Self::OpenUrl(field::string(compound, "url")?),
            "run_command" => Self::RunCommand(field::string(compound, "command")?),
            "suggest_command" => Self::SuggestCommand(field::string(compound, "command")?),
            "show_dialog" => Self::ShowDialog(field::required(compound, "dialog")?.clone()),
            "change_page" => {
                let page = field::int(compound, "page")?;
                if page < 1 {
                    // ExtraCodecs.POSITIVE_INT.
                    return Err(Error::UnknownVariant {
                        name: "ClickEvent page",
                        value: page.to_string(),
                    });
                }
                Self::ChangePage(page)
            }
            "copy_to_clipboard" => Self::CopyToClipboard(field::string(compound, "value")?),
            "custom" => Self::Custom {
                id: field::string(compound, "id")?,
                payload: compound.get("payload").cloned(),
            },
            other => {
                return Err(Error::UnknownVariant {
                    name: "ClickEvent action",
                    value: other.to_owned(),
                });
            }
        })
    }
}

/// What hovering over a component shows.
#[derive(Debug, Clone, PartialEq)]
pub enum HoverEvent<'a> {
    /// Show another component as a tooltip.
    ShowText(Box<Component<'a>>),
    /// Show an item's tooltip.
    ///
    /// The fields are `ItemStackTemplate.MAP_CODEC` inlined into the event
    /// compound rather than nested under a key of their own.
    ShowItem {
        /// Item registry id.
        id: Cow<'a, str>,
        /// Stack size, 1 to 99. Omitted from the wire when it is 1.
        count: i32,
        /// Data component patch, kept verbatim: decoding it needs the
        /// connection's data-component registry, which this crate has no view
        /// of. Absent and empty are the same thing, and both encode to absent.
        components: Option<Compound<'a>>,
    },
    /// Show an entity's tooltip.
    ShowEntity {
        /// Entity type registry id.
        id: Cow<'a, str>,
        /// Entity uuid, which travels as four big-endian ints.
        uuid: u128,
        /// Custom name to show above the type line.
        name: Option<Box<Component<'a>>>,
    },
}

/// `ExtraCodecs.intRange(1, 99)` in `ItemStackTemplate.MAP_CODEC`.
const MAX_STACK_COUNT: i32 = 99;

impl<'a> HoverEvent<'a> {
    /// The `action` discriminant.
    #[must_use]
    pub const fn action(&self) -> &'static str {
        match self {
            Self::ShowText(_) => "show_text",
            Self::ShowItem { .. } => "show_item",
            Self::ShowEntity { .. } => "show_entity",
        }
    }

    pub(super) fn to_tag(&self) -> Tag<'_> {
        let mut compound = Compound::new();
        compound.insert("action", borrowed_string(self.action()));
        match self {
            Self::ShowText(value) => {
                compound.insert("value", value.to_tag());
            }
            Self::ShowItem {
                id,
                count,
                components,
            } => {
                compound.insert("id", borrowed_string(id));
                if *count != 1 {
                    compound.insert("count", Tag::Int(*count));
                }
                if let Some(components) = components {
                    if !components.is_empty() {
                        compound.insert("components", Tag::Compound(components.clone()));
                    }
                }
            }
            Self::ShowEntity { id, uuid, name } => {
                compound.insert("id", borrowed_string(id));
                compound.insert("uuid", uuid_tag(*uuid));
                compound.insert_optional("name", name.as_ref().map(|name| name.to_tag()));
            }
        }
        Tag::Compound(compound)
    }

    pub(super) fn from_tag(tag: &Tag<'a>) -> Result<Self> {
        let compound = as_compound(tag, "hover_event")?;
        let action = field::string(compound, "action")?;
        Ok(match action.as_ref() {
            "show_text" => Self::ShowText(Box::new(Component::from_tag(field::required(
                compound, "value",
            )?)?)),
            "show_item" => {
                let count = field::int_or(compound, "count", 1)?;
                if !(1..=MAX_STACK_COUNT).contains(&count) {
                    return Err(Error::UnknownVariant {
                        name: "show_item count",
                        value: count.to_string(),
                    });
                }
                Self::ShowItem {
                    id: field::string(compound, "id")?,
                    count,
                    components: compound
                        .get("components")
                        .and_then(Tag::as_compound)
                        .filter(|components| !components.is_empty())
                        .cloned(),
                }
            }
            "show_entity" => Self::ShowEntity {
                id: field::string(compound, "id")?,
                uuid: read_uuid(field::required(compound, "uuid")?)?,
                name: compound
                    .get("name")
                    .map(|name| Component::from_tag(name).map(Box::new))
                    .transpose()?,
            },
            other => {
                return Err(Error::UnknownVariant {
                    name: "HoverEvent action",
                    value: other.to_owned(),
                });
            }
        })
    }
}

/// `UUIDUtil.uuidToIntArray`: the sixteen big-endian bytes, read as four ints.
fn uuid_tag<'a>(uuid: u128) -> Tag<'a> {
    let bytes = uuid.to_be_bytes();
    Tag::IntArray(
        (0..4)
            .map(|index| {
                let start = index * 4;
                i32::from_be_bytes([
                    bytes[start],
                    bytes[start + 1],
                    bytes[start + 2],
                    bytes[start + 3],
                ])
            })
            .collect(),
    )
}

/// `UUIDUtil.LENIENT_CODEC`: four ints, or a dashed string as a fallback.
fn read_uuid(tag: &Tag<'_>) -> Result<u128> {
    match tag {
        Tag::IntArray(values) if values.len() == 4 => {
            let mut bytes = [0u8; 16];
            for (index, value) in values.iter().enumerate() {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
            }
            Ok(u128::from_be_bytes(bytes))
        }
        Tag::String(value) => parse_dashed_uuid(value).ok_or_else(|| Error::UnknownVariant {
            name: "uuid",
            value: value.to_string(),
        }),
        other => Err(Error::WrongTagType {
            field: "uuid",
            expected: "TAG_Int_Array of four",
            found: other.id(),
        }),
    }
}

fn parse_dashed_uuid(value: &str) -> Option<u128> {
    let mut digits = String::with_capacity(32);
    for (index, part) in value.split('-').enumerate() {
        if index >= 5 {
            return None;
        }
        digits.push_str(part);
    }
    if digits.len() != 32 {
        return None;
    }
    u128::from_str_radix(&digits, 16).ok()
}

fn as_compound<'a, 'c>(tag: &'c Tag<'a>, field: &'static str) -> Result<&'c Compound<'a>> {
    tag.as_compound().ok_or_else(|| Error::WrongTagType {
        field,
        expected: "TAG_Compound",
        found: tag.id(),
    })
}
