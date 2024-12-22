//! Protocol encoding and decoding traits and error types for Minecraft networking.

use std::io::{Cursor, Write};

/// Error that can occur during protocol encoding
pub enum EncodeError<E> {
    /// Protocol-specific encoding error
    Encode(E),
    /// I/O error during encoding
    Io(std::io::Error),
}

/// Error that can occur during protocol decoding
pub enum DecodeError<E> {
    /// Protocol-specific decoding error
    Decode(E),
    /// I/O error during decoding
    Io(std::io::Error),
}

impl From<std::io::Error> for EncodeError<std::io::Error> {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Trait for types that can be encoded into a protocol format
pub trait Encode {
    /// The error type that can occur during encoding
    type Error;

    /// Encode this value into the given writer
    fn encode(&self, w: impl Write) -> Result<(), EncodeError<Self::Error>>;
}

/// Trait for types that can be decoded from a protocol format
pub trait Decode<'a> {
    /// The error type that can occur during decoding
    type Error;

    /// Decode a value from the given cursor
    fn decode(r: Cursor<&'a [u8]>) -> Result<Self, DecodeError<Self::Error>>
    where
        Self: Sized;
}
