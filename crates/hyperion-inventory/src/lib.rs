#![allow(missing_docs, reason = "todo: fix")]
//! Inventory management for Minecraft-like games.
//!
//! This crate provides inventory functionality similar to Minecraft, including:
//! - Fixed-size inventories with slots for items
//! - Hotbar and cursor management
//! - Item stacking and movement between slots
//! - Armor slots and offhand slot for player inventories

use std::cmp::min;

use flecs_ecs::{core::World, macros::Component, prelude::Module};
use roaring::RoaringBitmap;
use valence_protocol::{ItemKind, ItemStack};

/// Action-related inventory functionality
pub mod action;
/// Inventory parsing utilities
pub mod parser;

/// A player inventory with 46 slots
pub type PlayerInventory = Inventory<46>;

/// An inventory with a fixed number of slots for storing items.
///
/// The inventory tracks which slots have been updated and maintains a cursor position
/// for item movement operations.
#[derive(Component, Debug, PartialEq)]
pub struct Inventory<const T: usize> {
    /// The item slots in this inventory
    slots: [ItemStack; T],
    /// Current cursor/selected slot position
    hand_slot: u16,
    /// Tracks which slots were updated since last tick
    pub updated_since_last_tick: RoaringBitmap, // todo: maybe make this private
    /// Whether the cursor position was updated since last tick
    pub hand_slot_updated_since_last_tick: bool, // todo: maybe make this private
}

/// Result of attempting to add an item to an inventory
#[derive(Debug)]
pub struct AddItemResult {
    /// Any remaining items that could not be added
    pub remaining: Option<ItemStack>,
}

impl<const T: usize> Default for Inventory<T> {
    fn default() -> Self {
        Self {
            slots: [ItemStack::EMPTY; T],
            hand_slot: 0,
            updated_since_last_tick: RoaringBitmap::new(),
            hand_slot_updated_since_last_tick: false,
        }
    }
}

use hyperion_crafting::{Crafting2x2, CraftingRegistry};
use snafu::prelude::*;

/// Errors that can occur when accessing inventory slots
#[derive(Debug, Snafu)]
pub enum InventoryAccessError {
    /// The requested slot index was invalid
    #[snafu(display("Invalid slot index: {index}"))]
    InvalidSlot {
        /// The invalid slot index
        index: u16,
    },
}

/// Result of attempting to add an item to a specific slot
enum TryAddSlot {
    /// Item was completely added
    Complete,
    /// Item was partially added
    Partial,
    /// Item could not be added
    Skipped,
}

/// Starting slot index for the hotbar
const HAND_START_SLOT: u16 = 36;

impl<const N: usize> Inventory<N> {
    /// Sets the item in the given slot
    pub fn set(&mut self, index: u16, stack: ItemStack) -> Result<(), InventoryAccessError> {
        let item = self.get_mut(index)?;
        *item = stack;
        self.updated_since_last_tick.insert(u32::from(index));
        Ok(())
    }

    /// Returns an iterator over non-empty slots and their indices
    pub fn items(&self) -> impl Iterator<Item = (u16, &ItemStack)> + '_ {
        self.slots.iter().enumerate().filter_map(|(idx, item)| {
            if item.is_empty() {
                None
            } else {
                Some((u16::try_from(idx).unwrap(), item))
            }
        })
    }

    /// Returns a reference to all slots
    #[must_use]
    pub const fn slots(&self) -> &[ItemStack; N] {
        &self.slots
    }

    /// Empties all slots in the inventory
    pub fn clear(&mut self) {
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_empty() {
                continue;
            }
            *slot = ItemStack::EMPTY;
            self.updated_since_last_tick
                .insert(u32::try_from(idx).unwrap());
        }
    }

    /// Sets the cursor position to the given slot index
    pub fn set_cursor(&mut self, index: u16) {
        if self.hand_slot == index {
            return;
        }

        self.hand_slot = index;
        self.hand_slot_updated_since_last_tick = true;
    }

    /// Gets the item at the cursor position
    #[must_use]
    pub fn get_cursor(&self) -> &ItemStack {
        self.get_hand_slot(self.hand_slot).unwrap()
    }

    /// Gets the absolute slot index of the cursor position
    #[must_use]
    pub const fn get_cursor_index(&self) -> u16 {
        self.hand_slot + HAND_START_SLOT
    }

    /// Gets a mutable reference to the item at the cursor position
    pub fn get_cursor_mut(&mut self) -> &mut ItemStack {
        self.get_hand_slot_mut(self.hand_slot).unwrap()
    }

    /// Takes one item from the stack at the cursor position
    pub fn take_one_held(&mut self) -> ItemStack {
        // decrement the held item
        let held_item = self.get_cursor_mut();

        if held_item.is_empty() {
            return ItemStack::EMPTY;
        }

        held_item.count -= 1;

        ItemStack::new(held_item.item, 1, held_item.nbt.clone())
    }

    /// Gets a reference to the item in the given slot
    pub fn get(&self, index: u16) -> Result<&ItemStack, InventoryAccessError> {
        self.slots
            .get(usize::from(index))
            .ok_or(InventoryAccessError::InvalidSlot { index })
    }

    /// Gets a mutable reference to the item in the given slot
    pub fn get_mut(&mut self, index: u16) -> Result<&mut ItemStack, InventoryAccessError> {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return Err(InventoryAccessError::InvalidSlot { index });
        };

        // assume that the slot is updated
        self.updated_since_last_tick.insert(u32::from(index));

        Ok(slot)
    }

    /// Swaps the items in two slots
    pub fn swap(&mut self, index_a: u16, index_b: u16) {
        let index_a = usize::from(index_a);
        let index_b = usize::from(index_b);

        self.slots.swap(index_a, index_b);
    }

    /// Gets a reference to an item in a hotbar slot
    pub fn get_hand_slot(&self, idx: u16) -> Result<&ItemStack, InventoryAccessError> {
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get(idx)
    }

    /// Gets a mutable reference to an item in a hotbar slot
    pub fn get_hand_slot_mut(&mut self, idx: u16) -> Result<&mut ItemStack, InventoryAccessError> {
        const HAND_START_SLOT: u16 = 36;
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get_mut(idx)
    }

    /// Attempts to add an item to a specific slot, returning the result of the operation
    fn try_add_to_slot(
        &mut self,
        slot: u16,
        to_add: &mut ItemStack,
        can_add_to_empty: bool,
    ) -> Result<TryAddSlot, InventoryAccessError> {
        let max_stack_size: i8 = to_add.item.max_stack();

        let existing_stack = self.get_mut(slot)?;

        if existing_stack.is_empty() {
            return if can_add_to_empty {
                let new_count = min(to_add.count, max_stack_size);
                *existing_stack = to_add.clone().with_count(new_count);
                to_add.count -= new_count;
                self.updated_since_last_tick.insert(u32::from(slot));
                return if to_add.count > 0 {
                    Ok(TryAddSlot::Partial)
                } else {
                    Ok(TryAddSlot::Complete)
                };
            } else {
                Ok(TryAddSlot::Skipped)
            };
        }

        let stackable = existing_stack.item == to_add.item && existing_stack.nbt == to_add.nbt;

        if stackable && existing_stack.count < max_stack_size {
            let space_left = max_stack_size - existing_stack.count;

            return if to_add.count <= space_left {
                existing_stack.count += to_add.count;
                *to_add = ItemStack::EMPTY;
                self.updated_since_last_tick.insert(u32::from(slot));
                Ok(TryAddSlot::Complete)
            } else {
                existing_stack.count = max_stack_size;
                to_add.count -= space_left;
                self.updated_since_last_tick.insert(u32::from(slot));
                Ok(TryAddSlot::Partial)
            };
        }

        Ok(TryAddSlot::Skipped)
    }
}

impl PlayerInventory {
    /// Slot index for boots armor piece
    pub const BOOTS_SLOT: u16 = 8;
    /// Slot index for chestplate armor piece
    pub const CHESTPLATE_SLOT: u16 = 6;
    /// Slot index for helmet armor piece
    pub const HELMET_SLOT: u16 = 5;
    /// Starting slot index for the hotbar
    pub const HOTBAR_START_SLOT: u16 = 36;
    /// Slot index for leggings armor piece
    pub const LEGGINGS_SLOT: u16 = 7;
    /// Slot index for offhand item
    pub const OFFHAND_SLOT: u16 = OFFHAND_SLOT;

    /// Gets the result of the current 2x2 crafting grid contents
    #[must_use]
    pub fn crafting_result(&self, registry: &CraftingRegistry) -> ItemStack {
        let indices = core::array::from_fn::<u16, 4, _>(|i| (u16::try_from(i).unwrap() + 1));

        let mut min_count = i8::MAX;

        let items: Crafting2x2 = indices.map(|idx| {
            let stack = self.get(idx).unwrap();

            if stack.is_empty() {
                return ItemKind::Air;
            }

            min_count = min_count.min(stack.count);
            stack.item
        });

        let result = registry
            .get_result_2x2(items)
            .cloned()
            .unwrap_or(ItemStack::EMPTY);

        result
    }

    /// Sets an item in a hotbar slot
    pub fn set_hotbar(&mut self, idx: u16, stack: ItemStack) {
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return;
        }

        self.set(idx, stack).unwrap();
    }

    /// Sets the item in the offhand slot
    pub fn set_offhand(&mut self, stack: ItemStack) {
        self.set(Self::OFFHAND_SLOT, stack).unwrap();
    }

    /// Sets the helmet armor piece
    pub fn set_helmet(&mut self, stack: ItemStack) {
        self.set(Self::HELMET_SLOT, stack).unwrap();
    }

    /// Sets the chestplate armor piece
    pub fn set_chestplate(&mut self, stack: ItemStack) {
        self.set(Self::CHESTPLATE_SLOT, stack).unwrap();
    }

    /// Sets the leggings armor piece
    pub fn set_leggings(&mut self, stack: ItemStack) {
        self.set(Self::LEGGINGS_SLOT, stack).unwrap();
    }

    /// Sets the boots armor piece
    pub fn set_boots(&mut self, stack: ItemStack) {
        self.set(Self::BOOTS_SLOT, stack).unwrap();
    }

    /// Gets the helmet armor piece
    #[must_use]
    pub fn get_helmet(&self) -> &ItemStack {
        self.get(Self::HELMET_SLOT).unwrap()
    }

    /// Gets the chestplate armor piece
    #[must_use]
    pub fn get_chestplate(&self) -> &ItemStack {
        self.get(Self::CHESTPLATE_SLOT).unwrap()
    }

    /// Gets the leggings armor piece
    #[must_use]
    pub fn get_leggings(&self) -> &ItemStack {
        self.get(Self::LEGGINGS_SLOT).unwrap()
    }

    /// Gets the boots armor piece
    #[must_use]
    pub fn get_boots(&self) -> &ItemStack {
        self.get(Self::BOOTS_SLOT).unwrap()
    }

    /// Gets the item in the offhand slot
    #[must_use]
    pub fn get_offhand(&self) -> &ItemStack {
        self.get(Self::OFFHAND_SLOT).unwrap()
    }

    /// Attempts to add an item to the inventory, returning any remaining items that couldn't be added
    pub fn try_add_item(&mut self, mut item: ItemStack) -> AddItemResult {
        let mut result = AddItemResult { remaining: None };

        // Try to add to hot bar (36-45) first, then the rest of the inventory (9-35)
        // try to stack first
        for slot in (36..=44).chain(9..36) {
            let Ok(add_slot) = self.try_add_to_slot(slot, &mut item, false) else {
                unreachable!("try_add_item should always return Ok");
            };

            match add_slot {
                TryAddSlot::Complete => {
                    return result;
                }
                TryAddSlot::Partial | TryAddSlot::Skipped => {}
            }
        }

        // Try to add to hot bar (36-44) first, then the rest of the inventory (9-35)
        // now try to add to empty slots
        for slot in (36..=44).chain(9..36) {
            let Ok(add_slot) = self.try_add_to_slot(slot, &mut item, true) else {
                unreachable!("try_add_item should always return Ok");
            };

            match add_slot {
                TryAddSlot::Complete => {
                    return result;
                }
                TryAddSlot::Partial | TryAddSlot::Skipped => {}
            }
        }

        // If there's any remaining item, set it in the result
        if item.count > 0 {
            result.remaining = Some(item);
        }

        result
    }
}

/// Converts a hotbar index to an absolute slot index
#[must_use]
pub fn slot_index_from_hand(hand_idx: u8) -> u16 {
    const HAND_START_SLOT: u16 = 36;
    const HAND_END_SLOT: u16 = 45;

    let hand_idx = u16::from(hand_idx);
    let hand_idx = hand_idx + HAND_START_SLOT;

    if hand_idx >= HAND_END_SLOT {
        return 0;
    }

    hand_idx
}

// todo: not sure if this is correct
pub const OFFHAND_SLOT: u16 = 45;

#[derive(Component)]
pub struct InventoryModule;

impl Module for InventoryModule {
    fn module(world: &World) {
        world.component::<PlayerInventory>();
    }
}
