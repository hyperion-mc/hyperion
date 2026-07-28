//! An item as something other than an inventory slot carries it.

use crate::{
    Decode, Encode, Reader, Result, Writer,
    item::{DataComponentPatch, nbt::NbtScan},
    types::RegistryId,
};

/// An item stack that cannot be empty (`ItemStackTemplate`).
///
/// Distinct from [`ItemStack`](crate::item::ItemStack) in both shape and field
/// order: a slot writes its count first, because a count of zero is how the
/// protocol spells an empty slot and nothing follows it. A template has no
/// empty case at all, so it leads with the item and the count is an ordinary
/// field. Sending one where the other belongs swaps two `VarInt`s and produces
/// a plausible wrong item rather than an error.
///
/// This is what a `minecraft:item` particle carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemStackTemplate<'a> {
    /// `minecraft:item` registry id.
    pub item: RegistryId,
    /// How many items. Never zero; a template has no empty form.
    pub count: i32,
    /// Components layered over the item type's defaults.
    pub components: DataComponentPatch<'a>,
}

impl ItemStackTemplate<'_> {
    /// A single item of that type with no components.
    #[must_use]
    pub const fn single(item: RegistryId) -> Self {
        Self {
            item,
            count: 1,
            components: DataComponentPatch::empty(),
        }
    }

    /// Copy every borrowed component value so the stack outlives its buffer.
    #[must_use]
    pub fn into_owned(self) -> ItemStackTemplate<'static> {
        ItemStackTemplate {
            item: self.item,
            count: self.count,
            components: self.components.into_owned(),
        }
    }
}

impl Encode for ItemStackTemplate<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.item.encode(writer)?;
        writer.var_int(self.count);
        self.components.encode(writer)
    }
}

impl<'a> ItemStackTemplate<'a> {
    /// Read a template.
    ///
    /// Not a [`Decode`] impl: a component's value is not length-prefixed, so
    /// measuring one needs a scanner that can walk NBT. Same shape as
    /// [`crate::item::ItemStack::decode`], for the same reason.
    ///
    /// # Errors
    /// See [`DataComponentPatch::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Ok(Self {
            item: RegistryId::decode(reader)?,
            count: reader.var_int()?,
            components: DataComponentPatch::decode(reader, nbt)?,
        })
    }
}
