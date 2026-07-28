//! Protocol errors.

use std::fmt;

/// Result alias for protocol operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Something went wrong reading or writing the wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The buffer ended before the value did.
    UnexpectedEof {
        /// Bytes the reader still needed.
        needed: usize,
        /// Bytes actually left.
        available: usize,
    },
    /// A `VarInt` ran past its five-byte limit, or a `VarLong` past ten.
    VarIntTooLong,
    /// A length prefix was negative, which the protocol never emits.
    NegativeLength(i32),
    /// A collection field held more elements than the field permits.
    ListTooLong {
        /// Element count seen.
        length: usize,
        /// Maximum the field permits.
        max: usize,
    },
    /// A namespaced name used a character outside the permitted set.
    InvalidIdentifier(String),
    /// A string field exceeded the limit for that field.
    StringTooLong {
        /// Length seen, in UTF-16 code units to match the server's own check.
        length: usize,
        /// Maximum the field permits.
        max: usize,
    },
    /// String bytes were not valid UTF-8.
    InvalidUtf8,
    /// A discriminant had no meaning in this protocol version.
    InvalidEnum {
        /// Name of the enum the value was being decoded into.
        name: &'static str,
        /// The value that was not recognised.
        value: i32,
    },
    /// Bytes remained after decoding a packet that should have consumed all of them.
    TrailingBytes(usize),
    /// A value nested deeper than the decoder will follow.
    ///
    /// An item component can contain an item, so nesting is unbounded on the
    /// wire and a recursive decoder has to stop before the stack does.
    DepthLimitExceeded(u32),
    /// An item stack was empty where the field requires a real stack.
    EmptyItemStack,
    /// A block state id was outside the range this version has states for.
    ///
    /// Distinct from [`Error::InvalidEnum`] because the state registry is
    /// dense and version-specific rather than a named set: an id past the end
    /// usually means the sender is on a different game version, not that it
    /// sent something meaningless.
    InvalidBlockState {
        /// The id that named no state.
        value: i32,
    },
    /// A tag type byte named no NBT tag, or named `TAG_End` where a value was due.
    InvalidTagType(u8),
    /// An NBT value was a bare `TAG_End`, which `ByteBufCodecs.tagCodec` rejects.
    UnexpectedEndTag,
    /// NBT nested past `NbtAccounter`'s 512-deep limit.
    NbtTooDeep,
    /// An NBT string was malformed modified UTF-8, or held an unpaired surrogate.
    InvalidModifiedUtf8,
    /// A codec required a field the value did not carry.
    MissingField(&'static str),
    /// A field held a tag of the wrong type for its codec.
    WrongTagType {
        /// Field whose value was wrong.
        field: &'static str,
        /// Tag type the codec wanted.
        expected: &'static str,
        /// Tag type actually present.
        found: u8,
    },
    /// A string discriminant had no meaning in this protocol version.
    UnknownVariant {
        /// Name of the type the value was being decoded into.
        name: &'static str,
        /// The value that was not recognised.
        value: String,
    },
    /// No alternative in a `FuzzyCodec` matched the fields present.
    NoMatchingCodec(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { needed, available } => {
                write!(
                    f,
                    "unexpected end of input: needed {needed} bytes, {available} available"
                )
            }
            Self::VarIntTooLong => f.write_str("variable-length integer exceeded its maximum size"),
            Self::NegativeLength(n) => write!(f, "negative length prefix: {n}"),
            Self::StringTooLong { length, max } => {
                write!(f, "string too long: {length} characters, maximum {max}")
            }
            Self::ListTooLong { length, max } => {
                write!(f, "list too long: {length} elements, maximum {max}")
            }
            Self::InvalidIdentifier(value) => {
                write!(f, "not a valid namespaced identifier: {value}")
            }
            Self::InvalidUtf8 => f.write_str("string field was not valid UTF-8"),
            Self::InvalidEnum { name, value } => write!(f, "invalid {name} discriminant: {value}"),
            Self::TrailingBytes(n) => write!(f, "{n} trailing bytes after packet body"),
            Self::DepthLimitExceeded(max) => write!(f, "value nested deeper than {max} levels"),
            Self::EmptyItemStack => f.write_str("empty item stack where one was required"),
            Self::InvalidBlockState { value } => write!(f, "no block state has id {value}"),
            Self::InvalidTagType(id) => write!(f, "invalid NBT tag type: {id}"),
            Self::UnexpectedEndTag => f.write_str("NBT value was a bare TAG_End"),
            Self::NbtTooDeep => f.write_str("NBT nested deeper than 512 levels"),
            Self::InvalidModifiedUtf8 => f.write_str("NBT string was not valid modified UTF-8"),
            Self::MissingField(name) => write!(f, "missing required field: {name}"),
            Self::WrongTagType {
                field,
                expected,
                found,
            } => write!(
                f,
                "field {field} should be {expected}, found tag type {found}"
            ),
            Self::UnknownVariant { name, value } => write!(f, "invalid {name}: {value}"),
            Self::NoMatchingCodec(name) => write!(f, "no {name} matched the fields present"),
        }
    }
}

impl std::error::Error for Error {}
