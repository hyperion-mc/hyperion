// use valence_protocol::ItemStack;
//
// use super::{OFFHAND_SLOT, PlayerInventory, slot_index_from_hand};
//
// #[derive(Debug, PartialEq, Eq, Clone, Copy)]
// pub enum FullMouseButton {
// Left,
// Right,
// Middle,
// }
//
// #[derive(Debug, PartialEq, Eq, Clone, Copy)]
// pub enum MouseButton {
// Left,
// Right,
// }
//
// #[derive(Debug, PartialEq, Eq, Clone, Copy)]
// pub enum InventoryAction {
// NormalClick {
// button: MouseButton,
// slot: u16,
// },
// OutsideClick {
// button: MouseButton,
// },
// ShiftClick {
// button: MouseButton,
// slot: u16,
// },
// NumberKey {
// key: u8,
// slot: u16,
// },
//
// 'F' key is default bound to this
// OffhandSwap {
// slot: u16,
// },
// MiddleClick {
// slot: u16,
// },
//
// 'Q' key
// Drop,
// CtrlDrop,
//
// DragStart {
// button: FullMouseButton,
// },
// DragAdd {
// button: FullMouseButton,
// slot: u16,
// },
// DragEnd {
// button: FullMouseButton,
// },
//
// DoubleClick {
// slot: u16,
// },
// PickupAllReverse {
// slot: u16,
// },
// }
//
// pub struct InventoryAndCursor {
// pub inventory: PlayerInventory,
// pub cursor: ItemStack,
// }
//
// #[derive(Debug, PartialEq, Clone, Copy)]
// enum Amount {
// TrySingle,
// All,
// }
//
// fn take_one(item: &mut ItemStack) -> ItemStack {
// if item.is_empty() {
// return ItemStack::EMPTY;
// }
//
// item.count -= 1;
//
// ItemStack::new(item.item, 1, item.nbt.clone())
// }
//
// impl InventoryAndCursor {
// fn swap_cursor(&mut self, slot: u16, mode: Amount) {
// let in_inventory = self.inventory.get_mut(slot).unwrap();
//
// let cursor = &mut self.cursor;
//
// match mode {
// Amount::TrySingle => {
// if in_inventory.is_empty() {
// let single_cursor = take_one(cursor);
// in_inventory = single_cursor;
// } else {
// we must swap as we cannot just put one item down
// core::mem::swap(in_inventory, cursor);
// }
// }
// Amount::All => {
// swap the cursor and the item in the slot
// core::mem::swap(in_inventory, cursor);
// }
// }
// }
//
// #[expect(clippy::unused_self, unused, reason = "todo")]
// const fn drop_cursor(&self, mode: Amount) {
// todo: we will also need to figure out a way to create entity spawn events from this
// }
//
// pub fn apply(&mut self, action: InventoryAction) {
// match action {
// InventoryAction::NormalClick { button, slot } => {
// let mode = match button {
// MouseButton::Left => Amount::All,
// MouseButton::Right => Amount::TrySingle,
// };
//
// self.swap_cursor(slot, mode);
// }
// InventoryAction::OutsideClick { button } => {
// let mode = match button {
// MouseButton::Left => Amount::All,
// MouseButton::Right => Amount::TrySingle,
// };
//
// self.drop_cursor(mode);
// }
// InventoryAction::ShiftClick {
// button: MouseButton::Left | MouseButton::Right,
// ..
// } => {
// identical behavior so we combine branches
// }
// InventoryAction::NumberKey { key, slot } => {
// let other = slot_index_from_hand(key - 1);
// self.inventory.swap(slot, other);
// }
// InventoryAction::OffhandSwap { slot } => {
// self.inventory.swap(slot, OFFHAND_SLOT);
// }
// InventoryAction::MiddleClick { .. } => {
// unimplemented!("Middle click");
// }
// InventoryAction::Drop | InventoryAction::CtrlDrop => {
// Implement drop logic here
// self.drop_cursor(Amount::All);
// }
// InventoryAction::DragStart { .. } => {
// unimplemented!("Drag start");
// }
// InventoryAction::DragAdd { .. } => {
// unimplemented!("Drag add");
// }
// InventoryAction::DragEnd { .. } => {
// unimplemented!("Drag end");
// }
// InventoryAction::DoubleClick { .. } => {
// unimplemented!("Double click");
// }
// InventoryAction::PickupAllReverse { .. } => {
// unimplemented!("Pickup all reverse"); // impossible in vanilla
// }
// }
// }
// }
