#![feature(thread_local)]
use std::{ cell::Cell, cmp::min, num::Wrapping };

use derive_more::{ Deref, DerefMut };
use flecs_ecs::{ core::{ Entity, EntityViewGet, World }, macros::Component };
use valence_protocol::{
    packets::play::{ click_slot_c2s::ClickMode, open_screen_s2c::WindowType },
    ItemKind,
    ItemStack,
};

pub mod action;
pub mod parser;

pub type PlayerInventory = Inventory;

/// Placeholder; this will be added later.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Inventory {
    // doing this lets us create multiple pages from one inventory
    // size of the inventory window
    size: usize,
    // the slots in the inventory
    slots: Vec<ItemSlot>,
    hand_slot: u16,
    title: String,
    kind: WindowType,
    // how many times the inventory has been changed
    // used to determine if the client needs to be updated
    changed: Wrapping<u64>,
    readonly: bool,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct InventoryState {
    window_id: u8,
    state_id: Wrapping<i32>,
    // u64 is the last tick
    last_stack_clicked: (ItemStack, i64),
    last_button: (i8, i64),
    last_mode: (ClickMode, i64),
}

impl Default for InventoryState {
    fn default() -> Self {
        Self {
            window_id: 0,
            state_id: Wrapping(0),
            last_stack_clicked: (ItemStack::EMPTY, 0),
            last_button: (0, 0),
            last_mode: (ClickMode::Click, 0),
        }
    }
}

impl InventoryState {
    pub fn state_id(&self) -> i32 {
        self.state_id.0
    }

    pub fn increment_state_id(&mut self) {
        self.state_id += 1;
    }

    pub fn window_id(&self) -> u8 {
        self.window_id
    }

    pub fn set_window_id(&mut self) {
        self.window_id = non_zero_window_id();
    }

    pub fn last_stack_clicked(&self) -> (&ItemStack, i64) {
        (&self.last_stack_clicked.0, self.last_stack_clicked.1)
    }

    pub fn set_last_stack_clicked(&mut self, stack: ItemStack, tick: i64) {
        self.last_stack_clicked.0 = stack;
        self.last_stack_clicked.1 = tick;
    }

    pub fn last_button(&self) -> (i8, i64) {
        self.last_button
    }

    pub fn set_last_button(&mut self, button: i8, tick: i64) {
        self.last_button.0 = button;
        self.last_button.1 = tick;
    }

    pub fn last_mode(&self) -> (ClickMode, i64) {
        self.last_mode
    }

    pub fn set_last_mode(&mut self, mode: ClickMode, tick: i64) {
        self.last_mode.0 = mode;
        self.last_mode.1 = tick;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemSlot {
    pub readonly: bool,
    pub stack: ItemStack,
    pub changed: bool,
}

impl Default for ItemSlot {
    fn default() -> Self {
        Self {
            readonly: false,
            stack: ItemStack::EMPTY,
            changed: false,
        }
    }
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

#[derive(Component, Clone, PartialEq, Default, Debug, Deref, DerefMut)]
pub struct CursorItem(pub ItemStack);

#[derive(Debug)]
pub struct AddItemResult {
    pub remaining: Option<ItemStack>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            size: 46,
            slots: vec![ItemSlot::default(); 46],
            title: "Inventory".to_string(),
            kind: WindowType::Generic9x3,
            hand_slot: 0,
            changed: std::num::Wrapping(0),
            readonly: false,
        }
    }
}

use hyperion_crafting::{ Crafting2x2, CraftingRegistry };
use snafu::prelude::*;

#[derive(Debug, Snafu)]
pub enum InventoryAccessError {
    #[snafu(display("Invalid slot index: {index}"))] InvalidSlot {
        index: u16,
    },
}

enum TryAddSlot {
    Complete,
    Partial,
    Skipped,
}

const HAND_START_SLOT: u16 = 36;

impl Inventory {
    pub fn new(size: usize, title: String, kind: WindowType, readonly: bool) -> Self {
        Self {
            size,
            slots: vec![ItemSlot::default(); size],
            title,
            kind,
            hand_slot: 0,
            changed: std::num::Wrapping(0),
            readonly,
        }
    }

    pub fn increment_slot(&mut self, index: usize) {
        // set the slot as changed and increment the changed counter
        self.slots[index as usize].changed = true;
        self.changed += 1 << index;
    }

    pub fn changed(&self) -> u64 {
        self.changed.0
    }

    pub fn has_changed(&self) -> bool {
        self.changed.0 != 0
    }

    pub fn set_changed(&mut self, changed: u64) {
        self.changed.0 = changed;
    }

    pub fn kind(&self) -> WindowType {
        self.kind
    }

    pub fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn size(&self) -> usize {
        self.size
    }
    
    pub fn set(&mut self, index: u16, stack: ItemStack) -> Result<(), InventoryAccessError> {
        let index = usize::from(index);
        self.slots[index].stack = stack;
        self.increment_slot(index);
        // increment the changed countr
        Ok(())
    }

    pub fn items(&self) -> impl Iterator<Item = (u16, &ItemStack)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                if item.stack.is_empty() {
                    None
                } else {
                    Some((u16::try_from(idx).unwrap(), &item.stack))
                }
            })
    }

    #[must_use]
    pub const fn slots(&self) -> &Vec<ItemSlot> {
        &self.slots
    }

    pub const fn slots_mut(&mut self) -> &mut Vec<ItemSlot> {
        &mut self.slots
    }

    pub fn clear(&mut self) {
        let mut to_increment = Vec::new();
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.stack.is_empty() {
                continue;
            }
            slot.stack = ItemStack::EMPTY;
            to_increment.push(idx);
        }
        for idx in to_increment {
            self.increment_slot(idx);
        }
    }

    pub fn set_cursor(&mut self, index: u16) {
        if self.hand_slot == index {
            return;
        }

        self.hand_slot = index;
        self.increment_slot(index as usize);
    }

    #[must_use]
    pub fn get_cursor(&self) -> &ItemSlot {
        &self.slots[self.hand_slot as usize]
    }

    #[must_use]
    pub const fn get_cursor_index(&self) -> u16 {
        self.hand_slot + HAND_START_SLOT
    }

    // pub fn get_cursor_mut(&mut self) -> &mut ItemStack {
    // self.get_hand_slot_mut(self.hand_slot).unwrap()
    // }

    // pub fn take_one_held(&mut self) -> ItemStack {
    // decrement the held item
    // let held_item = &mut self.slots[usize::from(self.hand_slot)].stack;
    //
    // if held_item.is_empty() {
    // return ItemStack::EMPTY;
    // }
    //
    // held_item.count -= 1;
    //
    // ItemStack::new(held_item.item, 1, held_item.nbt.clone())
    // }

    pub fn get(&self, index: u16) -> Result<&ItemSlot, InventoryAccessError> {
        self.slots.get(usize::from(index)).ok_or(InventoryAccessError::InvalidSlot { index })
    }

    // pub fn get_mut(&mut self, index: u16) -> Result<&mut ItemSlot, InventoryAccessError> {
    // let Some(slot) = self.slots.get_mut(index as usize) else {
    // return Err(InventoryAccessError::InvalidSlot { index });
    // };
    //
    // assume that the slot is updated
    // self.increment_slot(index as usize);
    //
    // Ok(slot)
    // }

    pub fn swap(&mut self, index_a: u16, index_b: u16) {
        let index_a = usize::from(index_a);
        let index_b = usize::from(index_b);

        self.slots.swap(index_a, index_b);
    }

    pub fn get_hand_slot(&self, idx: u16) -> Result<&ItemSlot, InventoryAccessError> {
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get(idx)
    }

    pub fn get_hand_slot_mut(&mut self, idx: u16) -> Result<&mut ItemSlot, InventoryAccessError> {
        const HAND_START_SLOT: u16 = 36;
        const HAND_END_SLOT: u16 = 45;

        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        self.get_mut(idx)
    }

    pub fn get_mut(&mut self, index: u16) -> Result<&mut ItemSlot, InventoryAccessError> {
        let index = usize::from(index);
        self.increment_slot(index);
        Ok(&mut self.slots[index])
    }

    /// Returns remaining [`ItemStack`] if not all of the item was added to the slot
    fn try_add_to_slot(
        &mut self,
        slot: u16,
        to_add: &mut ItemStack,
        can_add_to_empty: bool
    ) -> Result<TryAddSlot, InventoryAccessError> {
        let max_stack_size: i8 = to_add.item.max_stack();

        let existing_stack = &mut self.slots[usize::from(slot)].stack;

        if existing_stack.is_empty() {
            return if can_add_to_empty {
                let new_count = min(to_add.count, max_stack_size);
                *existing_stack = to_add.clone().with_count(new_count);
                to_add.count -= new_count;
                self.increment_slot(slot as usize);
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
                self.increment_slot(slot as usize);
                Ok(TryAddSlot::Complete)
            } else {
                existing_stack.count = max_stack_size;
                to_add.count -= space_left;
                self.increment_slot(slot as usize);
                Ok(TryAddSlot::Partial)
            };
        }

        Ok(TryAddSlot::Skipped)
    }

    pub fn swap_slot(&mut self, slot: u16, other_slot: u16) {
        let slot = usize::from(slot);
        let other_slot = usize::from(other_slot);

        self.slots.swap(slot, other_slot);
        self.increment_slot(slot);
        self.increment_slot(other_slot);
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
        let indices = core::array::from_fn::<u16, 4, _>(|i| u16::try_from(i).unwrap() + 1);

        let mut min_count = i8::MAX;

        let items: Crafting2x2 = indices.map(|idx| {
            let stack = &self.get(idx).unwrap().stack;

            if stack.is_empty() {
                return ItemKind::Air;
            }

            min_count = min_count.min(stack.count);
            stack.item
        });

        let result = registry.get_result_2x2(items).cloned().unwrap_or(ItemStack::EMPTY);

        result
    }

    pub fn slots_inventory(&self) -> &[ItemSlot] {
        &self.slots[9..44]
    }

    pub fn slots_inventory_mut(&mut self) -> &mut [ItemSlot] {
        &mut self.slots[9..=44]
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
    pub fn get_helmet(&self) -> &ItemSlot {
        self.get(Self::HELMET_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_chestplate(&self) -> &ItemSlot {
        self.get(Self::CHESTPLATE_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_leggings(&self) -> &ItemSlot {
        self.get(Self::LEGGINGS_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_boots(&self) -> &ItemSlot {
        self.get(Self::BOOTS_SLOT).unwrap()
    }

    #[must_use]
    pub fn get_offhand(&self) -> &ItemSlot {
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

/// Thread-local non-zero id means that it will be very unlikely that one player will have two
/// of the same IDs at the same time when opening GUIs in succession.
///
/// We are skipping 0 because it is reserved for the player's inventory.
pub fn non_zero_window_id() -> u8 {
    #[thread_local]
    static ID: Cell<u8> = Cell::new(0);

    ID.set(ID.get().wrapping_add(1));

    if ID.get() == 0 {
        ID.set(1);
    }

    ID.get()
}
