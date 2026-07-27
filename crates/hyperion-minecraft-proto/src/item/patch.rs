//! The set of components added to, and removed from, an item's defaults.

use std::borrow::Cow;

use crate::{
    Encode, Error, Reader, Result, Writer,
    item::{ComponentType, nbt::NbtScan},
};

/// Components layered over an item's prototype.
///
/// This is `net.minecraft.core.component.DataComponentPatch`. An item on the
/// wire does not carry its full component map: it carries the difference from
/// the defaults its item type already implies, which is why removals are part
/// of the format rather than an absence.
///
/// Values are held as the bytes they occupied on the wire rather than as parsed
/// structures. That is deliberate. A proxy usually cares about one or two
/// components and must forward the rest untouched, and re-encoding from a parse
/// can only ever be as faithful as the parse. Holding the span makes an
/// unmodified component byte-identical on the way out by construction, not by
/// the parser happening to be right. Typed access is a layer on top: see
/// [`DataComponentPatch::get`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataComponentPatch<'a> {
    added: Vec<(ComponentType, Cow<'a, [u8]>)>,
    removed: Vec<ComponentType>,
}

impl<'a> DataComponentPatch<'a> {
    /// A patch that changes nothing.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
        }
    }

    /// True when the patch neither adds nor removes anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// Components this patch sets, in wire order, with their raw value bytes.
    #[must_use]
    pub fn added(&self) -> &[(ComponentType, Cow<'a, [u8]>)] {
        &self.added
    }

    /// Components this patch removes from the item's defaults, in wire order.
    #[must_use]
    pub fn removed(&self) -> &[ComponentType] {
        &self.removed
    }

    /// Raw value bytes for one component, if the patch sets it.
    #[must_use]
    pub fn raw(&self, kind: ComponentType) -> Option<&[u8]> {
        self.added
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, bytes)| bytes.as_ref())
    }

    /// Set a component from bytes already in its wire form.
    ///
    /// Replaces any existing value for the same type, keeping its position, and
    /// clears a pending removal of it: the server's map holds one entry per
    /// type that is either a value or a tombstone, never both.
    pub fn set_raw(&mut self, kind: ComponentType, value: impl Into<Cow<'a, [u8]>>) {
        self.removed.retain(|candidate| *candidate != kind);
        let value = value.into();
        if let Some(slot) = self
            .added
            .iter_mut()
            .find(|(candidate, _)| *candidate == kind)
        {
            slot.1 = value;
        } else {
            self.added.push((kind, value));
        }
    }

    /// Mark a component as removed from the item's defaults.
    pub fn remove(&mut self, kind: ComponentType) {
        self.added.retain(|(candidate, _)| *candidate != kind);
        if !self.removed.contains(&kind) {
            self.removed.push(kind);
        }
    }

    /// Read a patch, measuring each value against its type's wire shape.
    ///
    /// # Errors
    /// Returns an error on truncated input, on a component type id this
    /// protocol version does not define, or on a value that does not match the
    /// shape its type declares.
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Self::decode_at_depth(reader, nbt, 0)
    }

    pub(crate) fn decode_at_depth(
        reader: &mut Reader<'a>,
        nbt: &impl NbtScan,
        depth: u32,
    ) -> Result<Self> {
        if depth > crate::item::shape::MAX_DEPTH {
            return Err(Error::DepthLimitExceeded(crate::item::shape::MAX_DEPTH));
        }
        let added_count = count(reader.var_int()?, reader.remaining_len())?;
        let removed_count = count(reader.var_int()?, reader.remaining_len())?;

        let mut added = Vec::with_capacity(added_count.min(SANE_PREALLOC));
        for _ in 0..added_count {
            let kind = read_type(reader)?;
            let value = kind.shape().measure_at_depth(reader, nbt, depth + 1)?;
            added.push((kind, Cow::Borrowed(value)));
        }

        let mut removed = Vec::with_capacity(removed_count.min(SANE_PREALLOC));
        for _ in 0..removed_count {
            removed.push(read_type(reader)?);
        }

        Ok(Self { added, removed })
    }

    /// Copy every borrowed value so the patch no longer refers to the buffer.
    #[must_use]
    pub fn into_owned(self) -> DataComponentPatch<'static> {
        DataComponentPatch {
            added: self
                .added
                .into_iter()
                .map(|(kind, value)| (kind, Cow::Owned(value.into_owned())))
                .collect(),
            removed: self.removed,
        }
    }
}

/// `Reference2ObjectArrayMap` is sized from the declared count, and the server
/// clamps that allocation to 65536 entries. Doing the same keeps a bogus count
/// from turning into a huge allocation before the truncated read is noticed.
const SANE_PREALLOC: usize = 65536;

/// Reject a count that cannot fit in the bytes left, since a component type id
/// is at least one byte. Cheaper and stricter than discovering it element by
/// element.
fn count(value: i32, remaining: usize) -> Result<usize> {
    let value = usize::try_from(value).map_err(|_| Error::NegativeLength(value))?;
    if value > remaining {
        return Err(Error::UnexpectedEof {
            needed: value,
            available: remaining,
        });
    }
    Ok(value)
}

fn read_type(reader: &mut Reader<'_>) -> Result<ComponentType> {
    let id = reader.var_int()?;
    ComponentType::from_id(id).ok_or(Error::InvalidEnum {
        name: "data component type",
        value: id,
    })
}

impl Encode for DataComponentPatch<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let added = i32::try_from(self.added.len()).map_err(|_| Error::NegativeLength(-1))?;
        let removed = i32::try_from(self.removed.len()).map_err(|_| Error::NegativeLength(-1))?;
        writer.var_int(added);
        writer.var_int(removed);
        for (kind, value) in &self.added {
            writer.var_int(kind.id());
            writer.raw(value);
        }
        for kind in &self.removed {
            writer.var_int(kind.id());
        }
        Ok(())
    }
}

