//! Minecraft Java Edition wire protocol, generated from Mojang's own data.
//!
//! The version tables under [`generated`] come out of the vanilla data
//! generator and the decompiled server sources; the codecs under [`packets`]
//! are hand-written against those same sources. Nothing here depends on a
//! third-party protocol crate.
//!
//! Since Minecraft 26.1 the server ships unobfuscated, so every name in this
//! crate is Mojang's own name and greps directly against the server jar.

pub mod codec;
pub mod framing;
pub mod generated;
pub mod item;
pub mod nbt;
pub mod packets;
pub mod registry_data;
pub mod text;
pub mod world;

mod error;

pub use codec::{Reader, Writer};
pub use error::{Error, Result};
pub use generated::{MINECRAFT_VERSION, PROTOCOL_VERSION, WORLD_VERSION};

/// A value that can be written to the wire.
pub trait Encode {
    /// Append the wire representation of `self` to `writer`.
    ///
    /// # Errors
    /// Returns an error when a value violates a protocol limit, such as a
    /// string longer than the field permits.
    fn encode(&self, writer: &mut Writer) -> Result<()>;
}

/// A value that can be read from the wire.
pub trait Decode<'a>: Sized {
    /// Read one value from `reader`, advancing it past the bytes consumed.
    ///
    /// # Errors
    /// Returns an error on truncated input or a malformed encoding.
    fn decode(reader: &mut Reader<'a>) -> Result<Self>;
}
