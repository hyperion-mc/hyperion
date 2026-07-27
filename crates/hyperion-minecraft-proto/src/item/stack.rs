//! An item on the wire: a count, an item type, and a component patch.

use crate::{
    Encode, Error, Reader, Result, Writer,
    item::{DataComponentPatch, nbt::NbtScan},
};

/// One stack of items, as `net.minecraft.world.item.ItemStack` sends it.
///
/// The item type is its `minecraft:item` registry id rather than a name:
/// `Item.STREAM_CODEC` is `ByteBufCodecs.holderRegistry(Registries.ITEM)`,
/// which is a bare `VarInt` id with no inline form. Resolving it to a name
/// needs the registry, which is a concern above this layer;
/// [`crate::generated::registry::ITEM`] has the table for this version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemStack<'a> {
    /// How many items. Always at least one; an absent stack is [`Slot::Empty`].
    pub count: i32,
    /// `minecraft:item` registry id.
    pub item: i32,
    /// Components layered over the item type's defaults.
    pub components: DataComponentPatch<'a>,
}

/// An inventory slot, which may hold nothing.
///
/// The protocol has no separate "empty stack" encoding: a count of zero *is*
/// the empty slot, and no item id or components follow it. Modelling that as an
/// enum rather than as an `ItemStack` with `count == 0` keeps the impossible
/// state -- an empty stack that still carries components -- unrepresentable.
///
/// This is `ItemStack.OPTIONAL_STREAM_CODEC`. Fields typed as a plain
/// `ItemStack.STREAM_CODEC` reject the empty case; see
/// [`ItemStack::decode_non_empty`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Slot<'a> {
    /// Nothing in the slot.
    #[default]
    Empty,
    /// A stack of at least one item.
    Occupied(ItemStack<'a>),
}

impl<'a> ItemStack<'a> {
    /// Read a stack that is required to be non-empty.
    ///
    /// # Errors
    /// Returns [`Error::EmptyItemStack`] on a zero count, mirroring the throw
    /// in `ItemStack.STREAM_CODEC`, plus the errors of
    /// [`DataComponentPatch::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Self::decode_non_empty(reader, nbt)
    }

    /// Read a stack that is required to be non-empty.
    ///
    /// # Errors
    /// See [`ItemStack::decode`].
    pub fn decode_non_empty(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        match Slot::decode(reader, nbt)? {
            Slot::Occupied(stack) => Ok(stack),
            Slot::Empty => Err(Error::EmptyItemStack),
        }
    }

    /// Copy every borrowed component value so the stack outlives its buffer.
    #[must_use]
    pub fn into_owned(self) -> ItemStack<'static> {
        ItemStack {
            count: self.count,
            item: self.item,
            components: self.components.into_owned(),
        }
    }
}

impl<'a> Slot<'a> {
    /// Read a slot, which may be empty.
    ///
    /// # Errors
    /// See [`DataComponentPatch::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        let count = reader.var_int()?;
        // The server tests `count <= 0`, not `count == 0`, so a negative count
        // is an empty slot rather than an error. Matching that matters: a
        // stricter reader would reject a stream the server accepts.
        if count <= 0 {
            return Ok(Self::Empty);
        }
        let item = reader.var_int()?;
        let components = DataComponentPatch::decode(reader, nbt)?;
        Ok(Self::Occupied(ItemStack {
            count,
            item,
            components,
        }))
    }

    /// Copy every borrowed component value so the slot outlives its buffer.
    #[must_use]
    pub fn into_owned(self) -> Slot<'static> {
        match self {
            Self::Empty => Slot::Empty,
            Self::Occupied(stack) => Slot::Occupied(stack.into_owned()),
        }
    }
}

impl Encode for ItemStack<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        if self.count <= 0 {
            return Err(Error::EmptyItemStack);
        }
        writer.var_int(self.count);
        writer.var_int(self.item);
        self.components.encode(writer)
    }
}

impl Encode for Slot<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        match self {
            Self::Empty => {
                writer.var_int(0);
                Ok(())
            }
            Self::Occupied(stack) => stack.encode(writer),
        }
    }
}
