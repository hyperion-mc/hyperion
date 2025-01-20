#![feature(get_many_mut)]
#![feature(thread_local)]
use std::{cell::Cell, cmp::min, num::Wrapping};

use derive_more::{Deref, DerefMut};
use flecs_ecs::{core::Entity, macros::Component};
use tracing::debug;
use valence_protocol::{
    ItemKind, ItemStack,
    nbt::Compound,
    packets::play::{click_slot_c2s::ClickMode, open_screen_s2c::WindowType},
};

pub type PlayerInventory = Inventory;

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Inventory {
    /// The slots in the inventory
    slots: Box<[ItemSlot]>,
    /// Index to the slot held in the player's hand. This is guaranteed to be a valid index.
    hand_slot: u16,
    title: String,
    kind: WindowType,
    readonly: bool,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct InventoryState {
    window_id: u8,
    state_id: Wrapping<i32>,
    // i64 is the last tick
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
    #[must_use]
    pub const fn state_id(&self) -> i32 {
        self.state_id.0
    }

    pub fn increment_state_id(&mut self) {
        self.state_id += 1;
    }

    #[must_use]
    pub const fn window_id(&self) -> u8 {
        self.window_id
    }

    pub fn set_window_id(&mut self) {
        self.window_id = non_zero_window_id();
    }

    pub fn reset_window_id(&mut self) {
        self.window_id = 0;
    }

    #[must_use]
    pub const fn last_stack_clicked(&self) -> (&ItemStack, i64) {
        (&self.last_stack_clicked.0, self.last_stack_clicked.1)
    }

    pub fn set_last_stack_clicked(&mut self, stack: ItemStack, tick: i64) {
        self.last_stack_clicked.0 = stack;
        self.last_stack_clicked.1 = tick;
    }

    #[must_use]
    pub const fn last_button(&self) -> (i8, i64) {
        self.last_button
    }

    pub fn set_last_button(&mut self, button: i8, tick: i64) {
        self.last_button.0 = button;
        self.last_button.1 = tick;
    }

    #[must_use]
    pub const fn last_mode(&self) -> (ClickMode, i64) {
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

impl ItemSlot {
    #[must_use]
    pub fn new(item: ItemKind, count: i8, nbt: Option<Compound>, readonly: Option<bool>) -> Self {
        Self {
            readonly: readonly.unwrap_or(false),
            stack: ItemStack::new(item, count, nbt),
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
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
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
        Self::new(46, "Inventory".to_string(), WindowType::Generic9x3, false)
    }
}

use hyperion_crafting::{Crafting2x2, CraftingRegistry};
use snafu::prelude::*;

#[derive(Debug, Snafu)]
pub enum InventoryAccessError {
    #[snafu(display("Invalid slot index: {index}"))]
    InvalidSlot { index: u16 },
}

enum AddSlot {
    Complete,
    Partial,
    Skipped,
}

const HAND_START_SLOT: u16 = 36;
const HAND_END_SLOT: u16 = 45;

impl Inventory {
    #[must_use]
    pub fn new(size: usize, title: String, kind: WindowType, readonly: bool) -> Self {
        // TODO: calculate size from WindowType to avoid invalid states
        Self {
            slots: vec![ItemSlot::default(); size].into_boxed_slice(),
            title,
            kind,
            hand_slot: 36,
            readonly,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> WindowType {
        self.kind
    }

    #[must_use]
    pub const fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    #[must_use]
    pub const fn size(&self) -> usize {
        self.slots.len()
    }

    pub fn set(&mut self, index: u16, stack: ItemStack) -> Result<(), InventoryAccessError> {
        self.get_mut(index)?.stack = stack;
        Ok(())
    }

    pub fn set_slot(&mut self, index: u16, mut slot: ItemSlot) -> Result<(), InventoryAccessError> {
        slot.changed = true;
        *self.get_mut_maybe_change(index)? = slot;
        Ok(())
    }

    pub fn items(&self) -> impl Iterator<Item = (u16, &ItemStack)> + '_ {
        self.slots.iter().enumerate().filter_map(|(idx, item)| {
            if item.stack.is_empty() {
                None
            } else {
                Some((u16::try_from(idx).unwrap(), &item.stack))
            }
        })
    }

    #[must_use]
    pub const fn slots(&self) -> &[ItemSlot] {
        &self.slots
    }

    #[must_use]
    pub const fn slots_mut(&mut self) -> &mut [ItemSlot] {
        &mut self.slots
    }

    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            if slot.stack.is_empty() {
                continue;
            }
            slot.stack = ItemStack::EMPTY;
            slot.changed = true;
        }
    }

    pub fn set_cursor(&mut self, index: u16) -> Result<(), InventoryAccessError> {
        let index = self.hand_slot_index(index)?;
        if self.hand_slot == index {
            return Ok(());
        }

        debug!("Setting cursor to slot {}", index);

        // Mark the slot as changed
        self.get_mut(index)?;

        self.hand_slot = index;
        Ok(())
    }

    #[must_use]
    pub fn get_cursor(&self) -> &ItemSlot {
        self.get(self.hand_slot)
            .expect("hand_slot is a valid index")
    }

    #[must_use]
    pub const fn get_cursor_index(&self) -> u16 {
        self.hand_slot
    }

    pub fn get(&self, index: u16) -> Result<&ItemSlot, InventoryAccessError> {
        self.slots
            .get(usize::from(index))
            .ok_or(InventoryAccessError::InvalidSlot { index })
    }

    pub fn swap(&mut self, index_a: u16, index_b: u16) {
        let index_a = usize::from(index_a);
        let index_b = usize::from(index_b);

        self.slots.swap(index_a, index_b);
    }

    pub fn hand_slot_index(&self, idx: u16) -> Result<u16, InventoryAccessError> {
        let idx = idx + HAND_START_SLOT;

        if idx >= HAND_END_SLOT || usize::from(idx) >= self.size() {
            return Err(InventoryAccessError::InvalidSlot { index: idx });
        }

        Ok(idx)
    }

    pub fn get_hand_slot(&self, idx: u16) -> Result<&ItemSlot, InventoryAccessError> {
        self.get(self.hand_slot_index(idx)?)
    }

    pub fn get_hand_slot_mut(&mut self, idx: u16) -> Result<&mut ItemSlot, InventoryAccessError> {
        self.get_mut(self.hand_slot_index(idx)?)
    }

    pub fn get_mut(&mut self, index: u16) -> Result<&mut ItemSlot, InventoryAccessError> {
        let slot = self.get_mut_maybe_change(index)?;
        slot.changed = true;
        Ok(slot)
    }

    pub fn get_mut_maybe_change(
        &mut self,
        index: u16,
    ) -> Result<&mut ItemSlot, InventoryAccessError> {
        let index = usize::from(index);
        let slot = &mut self.slots[index];
        Ok(slot)
    }

    /// Returns remaining [`ItemStack`] if not all of the item was added to the slot
    fn add_to_slot(
        &mut self,
        slot: u16,
        to_add: &mut ItemStack,
        can_add_to_empty: bool,
    ) -> Result<AddSlot, InventoryAccessError> {
        let slot = self.get_mut_maybe_change(slot)?;
        let max_stack_size: i8 = to_add.item.max_stack();

        if slot.stack.is_empty() {
            return if can_add_to_empty {
                let new_count = min(to_add.count, max_stack_size);
                to_add.count -= new_count;
                slot.stack = to_add.clone().with_count(new_count);
                slot.changed = true;
                return if to_add.count > 0 {
                    Ok(AddSlot::Partial)
                } else {
                    Ok(AddSlot::Complete)
                };
            } else {
                Ok(AddSlot::Skipped)
            };
        }

        let stackable = slot.stack.item == to_add.item && slot.stack.nbt == to_add.nbt;

        if stackable && slot.stack.count < max_stack_size {
            let space_left = max_stack_size - slot.stack.count;

            return if to_add.count <= space_left {
                slot.stack.count += to_add.count;
                slot.changed = true;
                *to_add = ItemStack::EMPTY;
                Ok(AddSlot::Complete)
            } else {
                slot.stack.count = max_stack_size;
                slot.changed = true;
                to_add.count -= space_left;
                Ok(AddSlot::Partial)
            };
        }

        Ok(AddSlot::Skipped)
    }

    pub fn swap_slot(&mut self, slot: u16, other_slot: u16) {
        if slot == other_slot {
            return;
        }

        let slot = usize::from(slot);
        let other_slot = usize::from(other_slot);

        let [slot, other_slot] = self.slots.get_many_mut([slot, other_slot]).unwrap();
        std::mem::swap(slot, other_slot);
        slot.changed = true;
        other_slot.changed = true;
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

        let result = registry
            .get_result_2x2(items)
            .cloned()
            .unwrap_or(ItemStack::EMPTY);

        result
    }

    #[must_use]
    pub fn slots_inventory(&self) -> &[ItemSlot] {
        &self.slots[9..=44]
    }

    #[must_use]
    pub fn slots_inventory_mut(&mut self) -> &mut [ItemSlot] {
        &mut self.slots[9..=44]
    }

    pub fn set_hotbar(&mut self, idx: u16, stack: ItemStack) -> Result<(), InventoryAccessError> {
        self.get_hand_slot_mut(idx)?.stack = stack;
        Ok(())
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
        for can_add_to_empty in [false, true] {
            for slot in (36..=44).chain(9..36) {
                let add_slot = self
                    .add_to_slot(slot, &mut item, can_add_to_empty)
                    .expect("slot index is in bounds");

                match add_slot {
                    AddSlot::Complete => {
                        return result;
                    }
                    AddSlot::Partial | AddSlot::Skipped => {}
                }
            }
        }

        // If there's any remaining item, set it in the result
        if item.count > 0 {
            result.remaining = Some(item);
        }

        result
    }
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

pub trait ItemKindExt {
    fn is_helmet(&self) -> bool;
    fn is_chestplate(&self) -> bool;
    fn is_leggings(&self) -> bool;
    fn is_boots(&self) -> bool;
    fn is_armor(&self) -> bool;
}

impl ItemKindExt for ItemKind {
    fn is_helmet(&self) -> bool {
        matches!(
            self,
            Self::LeatherHelmet
                | Self::ChainmailHelmet
                | Self::IronHelmet
                | Self::GoldenHelmet
                | Self::DiamondHelmet
                | Self::NetheriteHelmet
                | Self::TurtleHelmet
                | Self::PlayerHead
        )
    }

    fn is_chestplate(&self) -> bool {
        matches!(
            self,
            Self::LeatherChestplate
                | Self::ChainmailChestplate
                | Self::IronChestplate
                | Self::GoldenChestplate
                | Self::DiamondChestplate
                | Self::NetheriteChestplate
        )
    }

    fn is_leggings(&self) -> bool {
        matches!(
            self,
            Self::LeatherLeggings
                | Self::ChainmailLeggings
                | Self::IronLeggings
                | Self::GoldenLeggings
                | Self::DiamondLeggings
                | Self::NetheriteLeggings
        )
    }

    fn is_boots(&self) -> bool {
        matches!(
            self,
            Self::LeatherBoots
                | Self::ChainmailBoots
                | Self::IronBoots
                | Self::GoldenBoots
                | Self::DiamondBoots
                | Self::NetheriteBoots
        )
    }

    fn is_armor(&self) -> bool {
        self.is_helmet() || self.is_chestplate() || self.is_leggings() || self.is_boots()
    }
}
