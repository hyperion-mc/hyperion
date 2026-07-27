//! Containers, equipment and held items.
//!
//! Every body here was declined by the generator for one reason: it carries an
//! `ItemStack`, and `ItemStack.OPTIONAL_STREAM_CODEC` branches on whether the
//! stack is empty. The stack itself is [`crate::item`]'s problem and is not
//! re-implemented here; these are the frames around it.
//!
//! # Why these do not implement [`Decode`](crate::Decode)
//!
//! Reading a stack back needs an [`NbtScan`], because a data component is
//! written with no length between its type id and its value, so the only way
//! past a value is to replay its shape and some shapes bottom out in an NBT
//! tag. `Decode::decode` takes a reader and nothing else, so each body here
//! has an inherent `decode` that takes the scanner too, exactly as
//! [`Slot::decode`] does. Encoding needs no such thing, so [`Encode`] is
//! derived and a server that only sends these never has to supply a scanner.
//!
//! The ids these bodies are sent under live in
//! [`crate::generated::packet_id::play::clientbound::PacketId`]; a body does
//! not carry its own id because the same body can appear under two of them.

use crate::{
    Encode, Error, Reader, Result, Writer,
    codec::read_count,
    item::{Slot, nbt::NbtScan},
};

/// `minecraft:container_set_content`, sent clientbound as play id 18.
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundContainerSetContentPacket#STREAM_CODEC`.
///
/// This is the whole-window resynchronisation: it replaces every slot at once,
/// so a client that has drifted is corrected by one of these rather than by a
/// run of [`ContainerSetSlot`].
#[derive(Debug, Clone, PartialEq, Eq, Encode)]
pub struct ContainerSetContent<'a> {
    /// Which window. `ByteBufCodecs.CONTAINER_ID`, a `VarInt` since 1.21.2 and
    /// an unsigned byte before it. Zero is the player's own inventory.
    #[proto(varint)]
    pub container_id: i32,
    /// The menu revision this content is for, echoed back by the next click so
    /// the server can tell a stale click from a fresh one.
    #[proto(varint)]
    pub state_id: i32,
    /// Every slot in the window, in slot order.
    /// `ItemStack.OPTIONAL_LIST_STREAM_CODEC`.
    pub items: Vec<Slot<'a>>,
    /// What the cursor is holding.
    pub carried_item: Slot<'a>,
}

impl<'a> ContainerSetContent<'a> {
    /// Read one body, using `nbt` to measure any tag inside an item.
    ///
    /// # Errors
    /// See [`Slot::decode`], plus [`Error::NegativeLength`] for a slot
    /// count that is not a count.
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        let container_id = reader.var_int()?;
        let state_id = reader.var_int()?;
        let items = decode_slots(reader, nbt)?;
        let carried_item = Slot::decode(reader, nbt)?;
        Ok(Self {
            container_id,
            state_id,
            items,
            carried_item,
        })
    }
}

/// `minecraft:container_set_slot`, sent clientbound as play id 20.
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket#STREAM_CODEC`.
#[derive(Debug, Clone, PartialEq, Eq, Encode)]
pub struct ContainerSetSlot<'a> {
    /// Which window, as in [`ContainerSetContent::container_id`].
    #[proto(varint)]
    pub container_id: i32,
    /// The menu revision, as in [`ContainerSetContent::state_id`].
    #[proto(varint)]
    pub state_id: i32,
    /// Index into that window's slots.
    pub slot: i16,
    /// What the slot now holds.
    pub item_stack: Slot<'a>,
}

impl<'a> ContainerSetSlot<'a> {
    /// Read one body, using `nbt` to measure any tag inside the item.
    ///
    /// # Errors
    /// See [`Slot::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Ok(Self {
            container_id: reader.var_int()?,
            state_id: reader.var_int()?,
            slot: reader.i16()?,
            item_stack: Slot::decode(reader, nbt)?,
        })
    }
}

/// `minecraft:set_cursor_item`, sent clientbound as play id 96.
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundSetCursorItemPacket#STREAM_CODEC`.
///
/// Before 1.21.2 the cursor was set with a [`ContainerSetSlot`] addressed to
/// window `-1`, slot `-1`. That spelling still parses but no longer means the
/// cursor, so a port that keeps it silently stops updating what the player is
/// dragging.
#[derive(Debug, Clone, PartialEq, Eq, Encode)]
pub struct SetCursorItem<'a> {
    /// What the cursor is holding.
    pub contents: Slot<'a>,
}

impl<'a> SetCursorItem<'a> {
    /// Read one body, using `nbt` to measure any tag inside the item.
    ///
    /// # Errors
    /// See [`Slot::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Ok(Self {
            contents: Slot::decode(reader, nbt)?,
        })
    }
}

/// `minecraft:set_player_inventory`, sent clientbound as play id 108.
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundSetPlayerInventoryPacket#STREAM_CODEC`.
///
/// The slot is an index into the player's own inventory rather than into
/// whatever window is open, which is what makes this different from a
/// [`ContainerSetSlot`] with container id zero: it lands correctly even while
/// a container has the player's slots remapped behind it.
#[derive(Debug, Clone, PartialEq, Eq, Encode)]
pub struct SetPlayerInventory<'a> {
    /// Index into `Inventory`, not into the open menu.
    #[proto(varint)]
    pub slot: i32,
    /// What that slot now holds.
    pub contents: Slot<'a>,
}

impl<'a> SetPlayerInventory<'a> {
    /// Read one body, using `nbt` to measure any tag inside the item.
    ///
    /// # Errors
    /// See [`Slot::decode`].
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        Ok(Self {
            slot: reader.var_int()?,
            contents: Slot::decode(reader, nbt)?,
        })
    }
}

/// Where a piece of equipment sits on an entity (`EquipmentSlot`).
///
/// `ClientboundSetEquipmentPacket` writes `equipmentSlot.ordinal()` rather than
/// going through `EquipmentSlot.STREAM_CODEC`, and reads it back as an index
/// into `EquipmentSlot.VALUES`, which is `List.of(values())`. So these numbers
/// are the declaration order in `net.minecraft.world.entity.EquipmentSlot` and
/// nothing else; they are not the ids any other packet uses for a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
// `u8` and not the `i32` the crate's `VarInt` enums use: this one is written
// as a single byte with a flag in its top bit, never as a `VarInt`.
#[repr(u8)]
pub enum EquipmentSlot {
    /// The held item.
    MainHand = 0,
    /// The off-hand item.
    OffHand = 1,
    /// Boots.
    Feet = 2,
    /// Leggings.
    Legs = 3,
    /// Chestplate.
    Chest = 4,
    /// Helmet.
    Head = 5,
    /// Animal body armour, such as a horse's or a wolf's.
    Body = 6,
    /// A saddle, which became an equipment slot in 1.21.5.
    Saddle = 7,
}

impl EquipmentSlot {
    /// Every slot, in the order that is also their wire encoding.
    pub const ALL: [Self; 8] = [
        Self::MainHand,
        Self::OffHand,
        Self::Feet,
        Self::Legs,
        Self::Chest,
        Self::Head,
        Self::Body,
        Self::Saddle,
    ];

    /// Resolve an ordinal, as the packet's own `EquipmentSlot.VALUES.get` does.
    #[must_use]
    pub const fn from_ordinal(ordinal: u8) -> Option<Self> {
        match ordinal {
            0 => Some(Self::MainHand),
            1 => Some(Self::OffHand),
            2 => Some(Self::Feet),
            3 => Some(Self::Legs),
            4 => Some(Self::Chest),
            5 => Some(Self::Head),
            6 => Some(Self::Body),
            7 => Some(Self::Saddle),
            _ => None,
        }
    }

    /// The ordinal this slot is written as.
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        self as u8
    }
}

/// `minecraft:set_equipment`, sent clientbound as play id 102.
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket#STREAM_CODEC`.
///
/// The entries are not length-prefixed. Each one is a slot byte whose top bit
/// says another entry follows, then that slot's stack, so the list ends with
/// the first byte that has the bit clear. An empty list therefore has no
/// encoding at all: vanilla's writer emits nothing after the entity id and its
/// reader would then consume the next packet's bytes looking for a slot, which
/// is why [`SetEquipment::encode`] refuses one instead of sending it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetEquipment<'a> {
    /// Whose equipment changed.
    pub entity: i32,
    /// The slots that changed, in any order, at least one.
    pub slots: Vec<(EquipmentSlot, Slot<'a>)>,
}

/// `ClientboundSetEquipmentPacket.CONTINUE_MASK`: set on a slot byte when
/// another entry follows it.
const CONTINUE_MASK: u8 = 0x80;

impl<'a> SetEquipment<'a> {
    /// Read one body, using `nbt` to measure any tag inside an item.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] for a slot ordinal this version
    /// does not define, plus the errors of [`Slot::decode`]. Truncated input
    /// fails rather than ending the list, matching the server's own loop.
    pub fn decode(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Self> {
        let entity = reader.var_int()?;
        let mut slots = Vec::new();
        loop {
            let byte = reader.u8()?;
            let ordinal = byte & !CONTINUE_MASK;
            let slot = EquipmentSlot::from_ordinal(ordinal).ok_or_else(|| Error::InvalidEnum {
                name: "equipment slot",
                value: i32::from(ordinal),
            })?;
            slots.push((slot, Slot::decode(reader, nbt)?));
            if byte & CONTINUE_MASK == 0 {
                return Ok(Self { entity, slots });
            }
        }
    }
}

impl Encode for SetEquipment<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let Some((last, rest)) = self.slots.split_last() else {
            // See the type's own docs: a zero-entry list is unreadable, not
            // merely useless, so it cannot be allowed onto the wire.
            return Err(Error::MissingField("set_equipment slots"));
        };
        writer.var_int(self.entity);
        for (slot, stack) in rest {
            writer.u8(slot.ordinal() | CONTINUE_MASK);
            stack.encode(writer)?;
        }
        writer.u8(last.0.ordinal());
        last.1.encode(writer)
    }
}

/// Read a `VarInt`-counted run of slots.
///
/// The capacity is clamped to the bytes actually present because the count is
/// attacker-controlled and an empty slot is one byte, so a count larger than
/// the remaining input is already a lie and must not reserve for itself.
fn decode_slots<'a>(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<Vec<Slot<'a>>> {
    let count = read_count(reader, None)?;
    let mut slots = Vec::with_capacity(count.min(reader.remaining_len()));
    for _ in 0..count {
        slots.push(Slot::decode(reader, nbt)?);
    }
    Ok(slots)
}
