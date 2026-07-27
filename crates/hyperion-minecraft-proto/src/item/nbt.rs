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
