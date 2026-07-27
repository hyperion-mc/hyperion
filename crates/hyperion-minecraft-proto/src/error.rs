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
            Self::InvalidUtf8 => f.write_str("string field was not valid UTF-8"),
            Self::InvalidEnum { name, value } => write!(f, "invalid {name} discriminant: {value}"),
            Self::TrailingBytes(n) => write!(f, "{n} trailing bytes after packet body"),
        }
    }
}

impl std::error::Error for Error {}
