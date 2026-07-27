//! The one thing item decoding needs from an NBT implementation.

use crate::Result;

/// Measures a network NBT tag without interpreting it.
///
/// Data components are not length-prefixed on the wire: `DataComponentPatch`'s
/// `STREAM_CODEC` writes a type id followed immediately by that type's value,
/// so the only way to find where a component ends is to walk its shape. Every
/// shape in the protocol bottoms out in primitives this crate already reads,
/// except the NBT tag, which needs a real NBT reader to measure. Rather than
/// grow a second NBT implementation here, the item layer takes that one
/// measurement as a parameter.
///
/// A network tag is the nameless form Mojang switched to in 1.20.2: a type
/// byte, then that type's payload, with no name between them. `TAG_End` (`0`)
/// stands for absent and occupies exactly that one byte.
pub trait NbtScan {
    /// Length in bytes of the network NBT tag at the start of `bytes`.
    ///
    /// The tag begins at index zero. Trailing bytes are expected and ignored:
    /// the caller is positioned mid-packet, not at a tag-sized buffer.
    ///
    /// # Errors
    /// Returns an error when the bytes are not a well-formed tag, including
    /// when the tag is truncated.
    fn tag_len(&self, bytes: &[u8]) -> Result<usize>;
}

impl<T: NbtScan + ?Sized> NbtScan for &T {
    fn tag_len(&self, bytes: &[u8]) -> Result<usize> {
        (**self).tag_len(bytes)
    }
}

/// [`NbtScan`] backed by this crate's own NBT reader.
///
/// The trait exists so the item layer does not depend on an NBT
/// implementation, not because this crate lacks one. Anything that already has
/// [`crate::nbt`] linked in wants this rather than a scanner of its own.
///
/// It measures by decoding and discarding, which is the only honest way to
/// find the end of a tag: the length is not written anywhere, so every byte
/// has to be walked whether or not the value is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scanner;

impl NbtScan for Scanner {
    fn tag_len(&self, bytes: &[u8]) -> Result<usize> {
        let mut reader = crate::Reader::new(bytes);
        // `decode_optional` rather than `Tag::decode`, because a bare
        // `TAG_End` is a legal one-byte value here: `FriendlyByteBuf.writeNbt`
        // writes it for a null tag, and a component whose shape is `Nbt` can
        // carry one. Rejecting it would fail a packet the server would accept.
        crate::nbt::decode_optional(&mut reader)?;
        Ok(bytes.len() - reader.remaining_len())
    }
}
