use std::{cmp::min, num::Wrapping};

use derive_more::{Deref, DerefMut};
use flecs_ecs::{
    core::{Entity, EntityViewGet, QueryBuilderImpl, SystemAPI, World, flecs},
    macros::{Component, system},
    prelude::Module,
};
use roaring::RoaringBitmap;
use valence_protocol::{
    ItemKind, ItemStack, VarInt,
    packets::play::{self, open_screen_s2c::WindowType},
};

pub mod action;
pub mod parser;

pub type PlayerInventory = Inventory;

/// Placeholder; this will be added later.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Inventory {
    size: usize,
    slots: Vec<ItemStack>,
    pub hand_slot: u16,
    pub title: String,
    pub kind: WindowType,
    pub changed: u64,
    pub readonly: bool,
    pub updated_since_last_tick: RoaringBitmap, // todo: maybe make this private
    pub hand_slot_updated_since_last_tick: bool, // todo: maybe make this private
}

#[derive(Component, Clone, Debug)]
pub struct OpenInventory {
    pub entity: Entity,
    pub client_changed: u64,
}

impl OpenInventory {
    pub fn new(entity: Entity) -> Self {
        Self {
            entity,
            client_changed: 0,
        }
    }
}

#[derive(Component, Debug)]
pub struct InventoryState {
    pub window_id: u8,
    pub state_id: Wrapping<i32>,
    pub slots_changed: u64,
    pub client_updated_cursor_item: Option<ItemStack>,
}

#[derive(Component, Clone, PartialEq, Default, Debug, Deref, DerefMut)]
pub struct CursorItem(pub ItemStack);

impl Default for InventoryState {
    fn default() -> Self {
        Self {
            window_id: 0,
            state_id: Wrapping(0),
            slots_changed: 0,
            client_updated_cursor_item: None,
        }
    }
}

#[derive(Debug)]
pub struct AddItemResult {
    pub remaining: Option<ItemStack>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            size: 46,
            slots: [ItemStack::EMPTY; 46].to_vec(),
            title: "Inventory".to_string(),
            kind: WindowType::Generic9x3,
            hand_slot: 0,
            changed: 0,
            updated_since_last_tick: RoaringBitmap::new(),
            hand_slot_updated_since_last_tick: false,
            readonly: false,
        }
    }
}

use hyperion_crafting::{Crafting2x2, CraftingRegistry};
use snafu::prelude::*;

#[derive(Debug, Snafu)]
pub enum InventoryAccessError {
    #[snafu(display("Invalid slot index: {index}"))]
    InvalidSlot { index: u16 },
}

enum TryAddSlot {
    Complete,
    Partial,
    Skipped,
}

const HAND_START_SLOT: u16 = 36;

impl Inventory {
    pub fn set(&mut self, index: u16, stack: ItemStack) -> Result<(), InventoryAccessError> {
        let item = self.get_mut(index)?;
        *item = stack;
        self.updated_since_last_tick.insert(u32::from(index));
        self.changed |= 1 << index;
        Ok(())
    }

    pub fn items(&self) -> impl Iterator<Item = (u16, &ItemStack)> + '_ {
        self.slots.iter().enumerate().filter_map(|(idx, item)| {
            if item.is_empty() {
                None
            } else {
                Some((u16::try_from(idx).unwrap(), item))
            }
        })
    }

    #[must_use]
    pub const fn slots(&self) -> &Vec<ItemStack> {
        &self.slots
    }

    pub fn clear(&mut self) {
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_empty() {
                continue;
            }
            *slot = ItemStack::EMPTY;
            self.updated_since_last_tick
                .insert(u32::try_from(idx).unwrap());
            self.changed |= 1 << idx;
        }
    }

    pub fn set_cursor(&mut self, index: u16) {
        if self.hand_slot == index {
            return;
        }

        self.hand_slot = index;
        self.hand_slot_updated_since_last_tick = true;
    }

    #[must_use]
    pub fn get_cursor(&self) -> &ItemStack {
        self.get_hand_slot(self.hand_slot).unwrap()
    }

    #[must_use]
    pub const fn get_cursor_index(&self) -> u16 {
        self.hand_slot + HAND_START_SLOT
    }

    pub fn get_cursor_mut(&mut self) -> &mut ItemStack {
        self.get_hand_slot_mut(self.hand_slot).unwrap()
    }

    pub fn take_one_held(&mut self) -> ItemStack {
        // decrement the held item
        let held_item = self.get_cursor_mut();

        if held_item.is_empty() {
            return ItemStack::EMPTY;
        }

        held_item.count -= 1;

        ItemStack::new(held_item.item, 1, held_item.nbt.clone())
    }

    pub fn get(&self, index: u16) -> Result<&ItemStack, InventoryAccessError> {
        self.slots
            .get(usize::from(index))
            .ok_or(InventoryAccessError::InvalidSlot { index })
    }

    pub fn get_mut(&mut self, index: u16) -> Result<&mut ItemStack, InventoryAccessError> {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return Err(InventoryAccessError::InvalidSlot { index });
        };

        // assume that the slot is updated
        self.updated_since_last_tick.insert(u32::from(index));
        self.changed |= 1 << index;

        Ok(slot)
    }

    pub fn swap(&mut self, index_a: u16, index_b: u16) {
        let index_a = usize::from(index_a);
        let index_b = usize::from(index_b);

        self.slots.swap(index_a, index_b);
    }

    pub fn get_hand_slot(&self, idx: u16) -> Result<&ItemStack, InventoryAccessError> {
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get(idx)
    }

    pub fn get_hand_slot_mut(&mut self, idx: u16) -> Result<&mut ItemStack, InventoryAccessError> {
        const HAND_START_SLOT: u16 = 36;
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get_mut(idx)
    }

    /// Returns remaining [`ItemStack`] if not all of the item was added to the slot
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
                self.changed |= 1 << slot;
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
                self.changed |= 1 << slot;
                Ok(TryAddSlot::Complete)
            } else {
                existing_stack.count = max_stack_size;
                to_add.count -= space_left;
                self.updated_since_last_tick.insert(u32::from(slot));
                self.changed |= 1 << slot;
                Ok(TryAddSlot::Partial)
            };
        }

        Ok(TryAddSlot::Skipped)
    }
}

impl PlayerInventory {
    pub const BOOTS_SLOT: u16 = 8;
    pub const CHESTPLATE_SLOT: u16 = 6;
    pub const HELMET_SLOT: u16 = 5;
    pub const HOTBAR_START_SLOT: u16 = 36;
    pub const LEGGINGS_SLOT: u16 = 7;
    pub const OFFHAND_SLOT: u16 = OFFHAND_SLOT;

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

    pub fn set_hotbar(&mut self, idx: u16, stack: ItemStack) {
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return;
        }

        self.set(idx, stack).unwrap();
    }

    pub fn set_offhand(&mut self, stack: ItemStack) {
        self.set(Self::OFFHAND_SLOT, stack).unwrap();
    }

    pub fn set_helmet(&mut self, stack: ItemStack) {
        self.set(Self::HELMET_SLOT, stack).unwrap();
    }

    pub fn set_chestplate(&mut self, stack: ItemStack) {
        self.set(Self::CHESTPLATE_SLOT, stack).unwrap();
    }

    pub fn set_leggings(&mut self, stack: ItemStack) {
        self.set(Self::LEGGINGS_SLOT, stack).unwrap();
    }

    pub fn set_boots(&mut self, stack: ItemStack) {
        self.set(Self::BOOTS_SLOT, stack).unwrap();
    }

    #[must_use]
    pub fn get_helmet(&self) -> &ItemStack {
        self.get(Self::HELMET_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_chestplate(&self) -> &ItemStack {
        self.get(Self::CHESTPLATE_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_leggings(&self) -> &ItemStack {
        self.get(Self::LEGGINGS_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_boots(&self) -> &ItemStack {
        self.get(Self::BOOTS_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_offhand(&self) -> &ItemStack {
        self.get(Self::OFFHAND_SLOT).unwrap()
    }

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

    pub fn slot(&self, idx: u16) -> &ItemStack {
        self.slots
            .get(idx as usize)
            .expect("slot index out of range")
    }

    pub fn set_slot<I: Into<ItemStack>>(&mut self, idx: u16, item: I) {
        let _unused = self.replace_slot(idx, item);
    }

    pub fn replace_slot<I: Into<ItemStack>>(&mut self, idx: u16, item: I) -> ItemStack {
        assert!(idx < self.slot_count(), "slot index of {idx} out of bounds");

        let new = item.into();
        let old = &mut self.slots[idx as usize];

        if new != *old {
            self.changed |= 1 << idx;
        }

        std::mem::replace(old, new)
    }

    pub fn slot_count(&self) -> u16 {
        self.slots.len() as u16
    }

    pub fn swap_slot(&mut self, idx_a: u16, idx_b: u16) {
        assert!(
            idx_a < self.slot_count(),
            "slot index of {idx_a} out of bounds"
        );
        assert!(
            idx_b < self.slot_count(),
            "slot index of {idx_b} out of bounds"
        );

        if idx_a == idx_b || self.slots[idx_a as usize] == self.slots[idx_b as usize] {
            // Nothing to do here, ignore.
            return;
        }

        self.changed |= 1 << idx_a;
        self.changed |= 1 << idx_b;

        self.slots.swap(idx_a as usize, idx_b as usize);
    }
}

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
