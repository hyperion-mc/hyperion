use std::{ borrow::Cow, mem::transmute };
use flecs_ecs::{
    core::{ flecs, EntityView, EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World },
    macros::{ observer, system, Component },
    prelude::Module,
};
use hyperion_inventory::{
    ItemKindExt,
    CursorItem,
    Inventory,
    InventoryState,
    ItemSlot,
    OpenInventory,
    PlayerInventory,
};
use hyperion_utils::EntityExt;
use tracing::debug;
use valence_protocol::{
    packets::play::{
        self,
        click_slot_c2s::{ ClickMode, SlotChange },
        entity_equipment_update_s2c::EquipmentEntry,
        ClickSlotC2s,
        UpdateSelectedSlotC2s,
    },
    Decode,
    VarInt,
};
use valence_server::ItemStack;
use valence_text::IntoText;

use super::{ event, handlers::PacketSwitchQuery, Player };
use crate::{ net::{ Compose, ConnectionId, DataBundle }, simulation::Position, storage };

#[derive(Component)]
pub struct InventoryModule;

impl Module for InventoryModule {
    fn module(world: &World) {
        world.component::<OpenInventory>();
        world.component::<InventoryState>();

        world.component::<Player>().add_trait::<(flecs::With, InventoryState)>();

        observer!(
            world,
            flecs::OnSet,
            &OpenInventory,
            &Compose($),
            &mut InventoryState,
            &CursorItem,
            &ConnectionId,
        ).each_iter(
            |it, row, (open_inventory, compose, inv_state, cursor_item, io)| {
                let system = it.system();
                let world = it.world();
                let entity = it.entity(row);
                let _entity_id = VarInt(entity.minecraft_id());
                let stream_id = *io;

                inv_state.set_window_id();

                open_inventory.entity
                    .entity_view(world)
                    .try_get::<&mut Inventory>(|inventory| {
                        let packet = &(play::OpenScreenS2c {
                            window_id: VarInt(inv_state.window_id() as i32),
                            window_type: inventory.kind(),
                            window_title: inventory.title().to_string().into_cow_text(),
                        });

                        compose.unicast(packet, stream_id, system).unwrap();

                        let packet = &(play::InventoryS2c {
                            window_id: inv_state.window_id(),
                            state_id: VarInt(inv_state.state_id()),
                            slots: Cow::Owned(
                                inventory
                                    .slots()
                                    .iter()
                                    .map(|slot| slot.stack.clone())
                                    .collect()
                            ),
                            carried_item: Cow::Borrowed(&cursor_item.0),
                        });

                        compose.unicast(packet, stream_id, system).unwrap();
                    })
                    .expect("open inventory: no inventory found");
            }
        );

        observer!(
            world,
            flecs::OnRemove,
            &OpenInventory,
            &Compose($),
            &mut InventoryState,
            &ConnectionId,
        ).each_iter(
            |it, row, (_open_inventory, compose, inv_state, io)| {
                let system = it.system();
                let _world = it.world();
                let entity = it.entity(row);
                let _entity_id = VarInt(entity.minecraft_id());
                let stream_id = *io;

                let packet = &(play::CloseScreenS2c {
                    window_id: inv_state.window_id(),
                });

                inv_state.reset_window_id();

                compose.unicast(packet, stream_id, system).unwrap();
            }
        );

        system!(
            "update_player_inventory",
            world,
            &Compose($),
            &mut PlayerInventory,
            &mut InventoryState,
            &Position,
            &CursorItem,
            ?&OpenInventory,
            &ConnectionId,
        )
            .multi_threaded()
            .kind::<flecs_ecs::prelude::flecs::pipeline::OnStore>()
            .each_iter(
                |
                    it,
                    row,
                    (compose, inventory, inv_state, position, cursor_item, open_inventory, io)
                | {
                    let system = it.system();
                    let world = it.world();
                    let entity = it.entity(row);
                    let entity_id = VarInt(entity.minecraft_id());
                    let stream_id = *io;

                    // update held item, offhand, and equipment
                    let mut equipment_changes: Vec<EquipmentEntry> = Vec::new();
                    let hand_slot = inventory.get_cursor_index();
                    for (idx, slot) in inventory.slots_mut().iter_mut().enumerate() {
                        if slot.changed {
                            if (idx as u16) == hand_slot {
                                debug!("cursor changed");
                                equipment_changes.push(EquipmentEntry {
                                    slot: 0,
                                    item: slot.stack.clone(),
                                });
                            }

                            if idx == 45 {
                                equipment_changes.push(EquipmentEntry {
                                    slot: 1,
                                    item: slot.stack.clone(),
                                });
                            }

                            if (5..=8).contains(&(idx as u16)) {
                                let index = match idx {
                                    5 => 5,
                                    6 => 4,
                                    7 => 3,
                                    8 => 2,
                                    _ => 0,
                                };
                                equipment_changes.push(EquipmentEntry {
                                    slot: index,
                                    item: slot.stack.clone(),
                                });
                            }
                        }
                    }

                    if !equipment_changes.is_empty() {
                        let packet = &(play::EntityEquipmentUpdateS2c {
                            entity_id,
                            equipment: equipment_changes,
                        });

                        compose
                            .broadcast_local(packet, position.to_chunk(), system)
                            .exclude(stream_id)
                            .send()
                            .unwrap();
                    }

                    let mut changed_slots: Vec<(ItemSlot, usize)> = Vec::new();
                    let mut inventories_mut: Vec<&mut ItemSlot> = Vec::new();
                    if let Some(open_inventory) = open_inventory {
                        open_inventory.entity.entity_view(world).get::<&mut Inventory>(|open_inv| {
                            (unsafe { transmute::<&mut Inventory, &mut Inventory>(open_inv) })
                                .slots_mut()
                                .iter_mut()
                                .for_each(|slot| inventories_mut.push(slot))
                        });
                    }

                    if inventories_mut.is_empty() {
                        inventory
                            .slots_mut()
                            .iter_mut()
                            .for_each(|slot| inventories_mut.push(slot));
                    } else {
                        inventory
                            .slots_inventory_mut()
                            .iter_mut()
                            .for_each(|slot| inventories_mut.push(slot));
                    }

                    for (idx, slot) in inventories_mut.iter_mut().enumerate() {
                        if slot.changed {
                            changed_slots.push((slot.clone(), idx));
                            slot.changed = false;
                        }
                    }

                    if !changed_slots.is_empty() {
                        let mut bundle = DataBundle::new(compose, system);
                        for (slot, idx) in changed_slots {
                            let packet = &(play::ScreenHandlerSlotUpdateS2c {
                                window_id: inv_state.window_id() as i8,
                                state_id: VarInt(inv_state.state_id()),
                                slot_idx: idx as i16,
                                slot_data: Cow::Borrowed(&slot.stack),
                            });

                            bundle.add_packet(packet).unwrap();
                        }

                        bundle.unicast(stream_id).unwrap();

                        let packet = &(play::ScreenHandlerSlotUpdateS2c {
                            window_id: -1,
                            state_id: VarInt(inv_state.state_id()),
                            slot_idx: -1,
                            slot_data: Cow::Borrowed(&cursor_item.0),
                        });

                        compose.unicast(packet, stream_id, system).unwrap();
                    }
                }
            );
    }
}

pub fn handle_update_selected_slot(
    packet: UpdateSelectedSlotC2s,
    query: &mut PacketSwitchQuery<'_>
) {
    if packet.slot > 8 {
        return;
    }

    debug!("slot: {}", packet.slot);
    // set the cursor slot from 35-44
    // 35 is the first slot in the hotbar
    let slot = packet.slot + 36;

    query.inventory.set_cursor(slot);

    let event = event::UpdateSelectedSlotEvent {
        client: query.id,
        slot: packet.slot as u8,
    };

    query.events.push(event, query.world);
}

pub fn handle_click_slot(packet: ClickSlotC2s<'_>, query: &mut PacketSwitchQuery<'_>) {
    // In here we need to handle different behaviors based on the click mode
    // First of we need to check if the player has the inventory open
    // Then we need to check if that inventory is readonly
    // If so then we need to resync the inventory with the client to make sure the client is in sync with the server

    debug!("slot_idx: {}", packet.slot_idx);
    debug!("button: {:?}", packet.button);
    debug!("mode: {:?}", packet.mode);

    query.id
        .entity_view(query.world)
        .get::<
            (&mut InventoryState, Option<&OpenInventory>, &mut PlayerInventory, &mut CursorItem)
        >(|(inv_state, open_inventory, player_inventory, cursor_item)| {
            let mut readonly = player_inventory.readonly();
            let mut inventories_mut: Vec<&mut ItemSlot> = vec![];
            let mut open_inv_size = 0;
            let mut player_only = true;
            if let Some(open_inventory) = open_inventory {
                open_inventory.entity.entity_view(query.world).get::<&mut Inventory>(|open_inv| {
                    readonly = open_inv.readonly();
                    open_inv_size = open_inv.size();
                    player_only = false;

                    // VERY UNSAFE
                    // IF SERVER CRASHES, THIS IS THE FIRST PLACE TO LOOK
                    (unsafe { transmute::<&mut Inventory, &mut Inventory>(open_inv) })
                        .slots_mut()
                        .iter_mut()
                        .for_each(|slot| inventories_mut.push(slot))
                });
            }

            if inventories_mut.is_empty() {
                player_inventory
                    .slots_mut()
                    .iter_mut()
                    .for_each(|slot| inventories_mut.push(slot));
            } else {
                player_inventory
                    .slots_inventory_mut()
                    .iter_mut()
                    .for_each(|slot| inventories_mut.push(slot));
            }

            // validate that packet_window_id is the same as the inv_state.window_id
            if packet.window_id != inv_state.window_id() {
                resync_inventory(
                    query.compose,
                    &query.system,
                    &inventories_mut
                        .iter()
                        .map(|slot| (*slot).clone())
                        .collect(),
                    inv_state,
                    cursor_item,
                    query.io_ref
                );
                return;
            }

            if packet.state_id != VarInt(inv_state.state_id()) {
                resync_inventory(
                    query.compose,
                    &query.system,
                    &inventories_mut
                        .iter()
                        .map(|slot| (*slot).clone())
                        .collect(),
                    inv_state,
                    cursor_item,
                    query.io_ref
                );
            }

            if readonly {
                resync_inventory(
                    query.compose,
                    &query.system,
                    &inventories_mut
                        .iter()
                        .map(|slot| (*slot).clone())
                        .collect(),
                    inv_state,
                    cursor_item,
                    query.io_ref
                );
                let event = storage::ClickSlotEvent {
                    window_id: inv_state.window_id(),
                    state_id: inv_state.state_id(),
                    slot_idx: packet.slot_idx as u16,
                    mode: packet.mode,
                    button: packet.button,
                    slot_changes: packet.slot_changes.to_vec(),
                    carried_item: cursor_item.0.clone(),
                };

                query.handlers.click.trigger_all(query, &event);

                return;
            }
            // button 0 is left click
            // button 1 is right click
            // button 2 is middle click

            let mut slots_changed: Vec<usize> = vec![];

            match packet.mode {
                // if the mode is click, and the on is 0, then check if its the same item as the cursor item
                // if it is, check how many items are in the slot
                // if its less than 64, then add as many items from the cursor item as possible till the count of the slot is 64
                // if the slot is empty, then move the cursor item to the slot
                // if its not the same item, then swap the cursor item with the slot item
                // if the slot_idx is -999 that means the cursor item is being dropped

                // if the mode is click, and the button is 1, then check if its the same item as the cursor item
                // if it is the same or the slot is empty, then move 1 item from the cursor item to the slot
                // if the cursor item is empty, then take half of the stack from the slot
                ClickMode::Click => {
                    let slot_idx = packet.slot_idx as u16;
                    match packet.button {
                        0 => {
                            handle_left_click_slot(
                                packet,
                                query,
                                &mut inventories_mut,
                                inv_state,
                                cursor_item,
                                &mut slots_changed,
                                slot_idx as i16,
                                player_only
                            );
                        }
                        1 => {
                            handle_right_click_slot(
                                packet,
                                query,
                                &mut inventories_mut,
                                cursor_item,
                                &mut slots_changed,
                                player_only
                            );
                        }
                        2 => {
                            // nothing yet
                            debug!("middle click");
                        }
                        _ => {}
                    }
                }
                ClickMode::Drag => {
                    //debug!("Slot Changed: {:?}", packet.slot_changes);
                    // We iterate through the slot changes,
                    // if slots changed is empty return
                    // if the button is 2 it means the player dragged with left click
                    // so we need to split the cursor item into the slots equally and the remainder stays in the cursor
                    // if the button is 6 it means the player dragged with right click
                    // so we need to put 1 item from the cursor into each slot and the remainder stays in the cursor
                    // also double check if the item in the slot is the same as the cursor item

                    if packet.slot_changes.is_empty() || cursor_item.0.is_empty() {
                        return;
                    }

                    let mut cursor = cursor_item.0.clone();
                    let slots = packet.slot_changes.clone();

                    match packet.button {
                        2 => {
                            handle_left_drag_slot(
                                &mut cursor,
                                slots,
                                &mut inventories_mut,
                                &mut slots_changed,
                                player_only
                            );
                        }
                        6 => {
                            handle_right_drag_slot(
                                &mut cursor,
                                slots,
                                &mut inventories_mut,
                                &mut slots_changed,
                                player_only
                            );
                        }
                        _ => {}
                    }

                    // Update the cursor with any remaining count
                    cursor_item.0 = cursor;
                }
                ClickMode::DoubleClick => {
                    handle_double_click(
                        packet,
                        &mut inventories_mut,
                        inv_state,
                        cursor_item,
                        &mut slots_changed,
                        player_only
                    );
                }
                ClickMode::ShiftClick => {
                    handle_shift_click(
                        packet,
                        &mut inventories_mut,
                        open_inv_size,
                        &mut slots_changed,
                        player_only
                    );
                }
                ClickMode::Hotbar => {
                    handle_hotbar_swap(
                        packet,
                        &mut inventories_mut,
                        open_inv_size,
                        &mut slots_changed,
                        player_only
                    );
                }
                ClickMode::CreativeMiddleClick => {
                    debug!("creative middle click");
                }
                ClickMode::DropKey => {
                    handle_drop_key(
                        packet,
                        query,
                        &mut inventories_mut,
                        cursor_item,
                        &mut slots_changed,
                        player_only
                    );
                }
            }

            resync_inventory(
                query.compose,
                &query.system,
                &inventories_mut
                    .iter()
                    .map(|slot| (*slot).clone())
                    .collect(),
                inv_state,
                cursor_item,
                query.io_ref
            );

            if let Some(open_inventory) = open_inventory {
                open_inventory.entity.entity_view(query.world).get::<&mut Inventory>(|open_inv| {
                    for &idx in &slots_changed {
                        if idx < open_inv_size {
                            open_inv.increment_slot(idx);
                        } else {
                            player_inventory.increment_slot(idx - open_inv_size);
                        }
                    }
                });
            } else {
                for &idx in &slots_changed {
                    player_inventory.increment_slot(idx);
                }
            }

            inv_state.set_last_button(0, query.compose.global().tick);
            inv_state.set_last_mode(ClickMode::Click, query.compose.global().tick);
        });
}

fn handle_left_click_slot(
    packet: ClickSlotC2s<'_>,
    query: &mut PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    inv_state: &mut InventoryState,
    cursor_item: &mut CursorItem,
    slots_changed: &mut Vec<usize>,
    slot_idx: i16,
    player_only: bool
) {
    if packet.slot_idx == -999 {
        if cursor_item.0.is_empty() {
            return;
        }
        let event = event::DropItemStackEvent {
            client: query.id,
            from_slot: None,
            item: cursor_item.0.clone(),
        };
        cursor_item.0 = ItemStack::EMPTY;
        query.events.push(event, query.world);
        return;
    }

    if packet.slot_idx < 0 || packet.slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    if player_only {
        if (5..=8).contains(&packet.slot_idx) && !cursor_item.0.is_empty() {
            let is_valid = match slot_idx {
                5 => cursor_item.0.item.is_helmet(),
                6 => cursor_item.0.item.is_chestplate(),
                7 => cursor_item.0.item.is_leggings(),
                8 => cursor_item.0.item.is_boots(),
                _ => false,
            };

            if !is_valid {
                return;
            }
        }
    }

    let slot = inventories_mut[slot_idx as usize].clone();
    if slot.readonly {
        return;
    }
    let cursor = cursor_item.0.clone();
    let slot_index = slot_idx as usize;

    if slot.stack.is_empty() {
        inventories_mut[slot_index].stack = cursor;
        inventories_mut[slot_index].changed = true;
        cursor_item.0 = ItemStack::EMPTY;
        slots_changed.push(slot_index);
        inv_state.set_last_stack_clicked(ItemStack::EMPTY, query.compose.global().tick);
    } else if slot.stack.item == cursor.item {
        let count = slot.stack.count.saturating_add(cursor.count);
        let max = slot.stack.item.max_stack();

        if count > max {
            let diff = count - max;
            inventories_mut[slot_index].stack = ItemStack::new(
                cursor.item,
                max,
                cursor.nbt.clone()
            );
            cursor_item.0 = ItemStack::new(cursor.item, diff, cursor.nbt.clone());
        } else {
            inventories_mut[slot_index].stack = ItemStack::new(
                cursor.item,
                count,
                cursor.nbt.clone()
            );
            cursor_item.0 = ItemStack::EMPTY;
        }

        inventories_mut[slot_index].changed = true;
        slots_changed.push(slot_index);
        inv_state.set_last_stack_clicked(
            inventories_mut[slot_index].stack.clone(),
            query.compose.global().tick
        );
    } else {
        let old_slot_stack = slot.stack.clone();
        inventories_mut[slot_index].stack = cursor;
        inventories_mut[slot_index].changed = true;
        cursor_item.0 = old_slot_stack.clone();
        slots_changed.push(slot_index);
        inv_state.set_last_stack_clicked(old_slot_stack, query.compose.global().tick);
    }
}

fn handle_right_click_slot(
    packet: ClickSlotC2s<'_>,
    query: &mut PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    cursor_item: &mut CursorItem,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    // Handle click outside inventory
    if packet.slot_idx == -999 {
        if !cursor_item.0.is_empty() {
            let new_stack = ItemStack::new(cursor_item.0.item, 1, cursor_item.0.nbt.clone());
            cursor_item.0.count = cursor_item.0.count.saturating_sub(1);
            if cursor_item.0.count == 0 {
                cursor_item.0 = ItemStack::EMPTY;
            }
            query.events.push(
                event::DropItemStackEvent {
                    client: query.id,
                    from_slot: None,
                    item: new_stack,
                },
                query.world
            );
        }
        return;
    }

    if packet.slot_idx < 0 || packet.slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    if player_only {
        let slot_idx = packet.slot_idx as i16;
        if (5..=8).contains(&slot_idx) && !cursor_item.0.is_empty() {
            let is_valid = match slot_idx {
                5 => cursor_item.0.item.is_helmet(),
                6 => cursor_item.0.item.is_chestplate(),
                7 => cursor_item.0.item.is_leggings(),
                8 => cursor_item.0.item.is_boots(),
                _ => false,
            };

            if !is_valid {
                return;
            }
        }
    }

    let slot_idx = packet.slot_idx as usize;
    let slot = &mut inventories_mut[slot_idx];
    let mut changed = false;

    if cursor_item.0.is_empty() {
        if !slot.stack.is_empty() && !slot.readonly {
            let total = slot.stack.count;
            let take = (total + 1) / 2; // Round up
            let leave = total - take;

            cursor_item.0 = ItemStack::new(slot.stack.item, take, slot.stack.nbt.clone());

            if leave > 0 {
                slot.stack.count = leave;
            } else {
                slot.stack = ItemStack::EMPTY;
            }
            changed = true;
        }
    } else {
        if slot.stack.is_empty() && !slot.readonly {
            slot.stack = ItemStack::new(cursor_item.0.item, 1, cursor_item.0.nbt.clone());
            cursor_item.0.count = cursor_item.0.count.saturating_sub(1);
            if cursor_item.0.count == 0 {
                cursor_item.0 = ItemStack::EMPTY;
            }
            changed = true;
        } else if
            slot.stack.item == cursor_item.0.item &&
            slot.stack.nbt == cursor_item.0.nbt &&
            slot.stack.count < slot.stack.item.max_stack()
            && !slot.readonly
        {
            slot.stack.count = slot.stack.count.saturating_add(1);
            cursor_item.0.count = cursor_item.0.count.saturating_sub(1);
            if cursor_item.0.count == 0 {
                cursor_item.0 = ItemStack::EMPTY;
            }
            changed = true;
        }
    }

    if changed {
        slot.changed = true;
        slots_changed.push(slot_idx);
    }
}

fn handle_left_drag_slot(
    cursor: &mut ItemStack,
    slots: Cow<'_, [SlotChange]>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    let total = cursor.count;
    let slots_len = slots.len() as i8;

    let per_slot = total / slots_len;
    let mut remainder = total % slots_len;

    if player_only {
        let mut slots = slots.iter().map(|slot| slot.idx as i16);
        if slots.any(|slot| (5..=8).contains(&slot)) {
            return;
        }
    }

    for slot_update in slots.iter() {
        if slot_update.idx < 0 || slot_update.idx >= (inventories_mut.len() as i16) {
            continue;
        }

        let slot_idx = slot_update.idx as usize;
        let mut stack = inventories_mut[slot_idx].stack.clone();

        if inventories_mut[slot_idx].readonly {
            continue;
        }

        // If the slot is empty, set both item and nbt, then count
        if stack.is_empty() {
            let available_space = cursor.item.max_stack();
            let to_add = per_slot.min(available_space);
            if to_add > 0 {
                stack.item = cursor.item;
                stack.nbt = cursor.nbt.clone();
                stack.count = to_add;
            }
            // Track remainder if not all per_slot could fit
            remainder = remainder.saturating_add(per_slot - to_add);
        } else if
            // If the slot is not empty but matches cursor item + nbt
            stack.item == cursor.item &&
            stack.nbt == cursor.nbt
        {
            let available_space = stack.item.max_stack() - stack.count;
            let to_add = per_slot.min(available_space);
            stack.count = stack.count.saturating_add(to_add);
            // Track remainder if not all per_slot could fit
            remainder = remainder.saturating_add(per_slot - to_add);
        }

        // Update the slot and mark it changed if any addition happened
        if stack != inventories_mut[slot_idx].stack && !inventories_mut[slot_idx].readonly {
            inventories_mut[slot_idx].stack = stack;
            inventories_mut[slot_idx].changed = true;
            slots_changed.push(slot_idx);
        }
    }
    // Update cursor to leftover remainder
    cursor.count = remainder;
}

fn handle_right_drag_slot(
    cursor: &mut ItemStack,
    slots: Cow<'_, [SlotChange]>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    if player_only {
        let mut slots = slots.iter().map(|slot| slot.idx as i16);
        if slots.any(|slot| (5..=8).contains(&slot)) {
            return;
        }
    }

    for slot_update in slots.iter() {
        if cursor.count == 0 {
            break;
        }

        if slot_update.idx < 0 || slot_update.idx >= (inventories_mut.len() as i16) {
            continue;
        }
        let slot_idx = slot_update.idx as usize;
        let mut stack = inventories_mut[slot_idx].stack.clone();

        if inventories_mut[slot_idx].readonly {
            continue;
        }

        if stack.is_empty() {
            stack.item = cursor.item;
            stack.nbt = cursor.nbt.clone();
            stack.count = 1;
            cursor.count -= 1;
        } else if
            // If the slot is not empty but matches cursor item + nbt
            stack.item == cursor.item &&
            stack.nbt == cursor.nbt
        {
            let available_space = stack.item.max_stack() - stack.count;
            let to_add = (1).min(available_space);
            stack.count = stack.count.saturating_add(to_add);
            cursor.count -= to_add;
        }

        // Update the slot and mark it changed if any addition happened
        if stack != inventories_mut[slot_idx].stack && !inventories_mut[slot_idx].readonly {
            inventories_mut[slot_idx].stack = stack;
            inventories_mut[slot_idx].changed = true;
            slots_changed.push(slot_idx);
        }
    }
}

fn handle_double_click(
    packet: ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    inv_state: &InventoryState,
    cursor_item: &mut CursorItem,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    // if the slot is empty... check if the last stack clicked is the same as the cursor item
    // ignoring the count
    // and also see if the last tick was within 1 tick of the current tick
    // if so, then try to take any matching items from the cursor item and add it to the
    // count of cursor item till it reaches 64 or there are no more matching items
    // make sure the slot is empty as well

    if packet.slot_idx < 0 || packet.slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    let slot_idx = packet.slot_idx as u16;
    let slot = inventories_mut[slot_idx as usize].clone();
    let cursor = cursor_item.0.clone();

    if slot.readonly {
        return;
    }

    if slot.stack.is_empty() {
        let last_stack = inv_state.last_stack_clicked();
        if last_stack.0.item == cursor.item && last_stack.0.nbt == cursor.nbt {
            let max_stack = cursor_item.0.item.max_stack();
            let mut needed = max_stack - cursor_item.0.count;

            // Skip if cursor is already at max
            if needed <= 0 {
                return;
            }

            // Collect matching slots with their counts
            let mut matching_slots: Vec<(usize, i8)> = inventories_mut
                .iter()
                .enumerate()
                .filter(|(_, slot)| {
                    slot.stack.item == cursor_item.0.item && slot.stack.nbt == cursor_item.0.nbt
                })
                .map(|(idx, slot)| (idx, slot.stack.count))
                .collect();

            // Sort by count ascending and index
            matching_slots.sort_by_key(|&(idx, count)| (count, idx));
            // Iterate through all slots
            for (idx, available) in matching_slots {
                let take = available.min(needed);

                // Update slot
                let slot = &mut inventories_mut[idx];
                if slot.readonly {
                    continue;
                }
                slot.stack.count -= take;
                if slot.stack.count == 0 {
                    slot.stack = ItemStack::EMPTY;
                }
                slot.changed = true;
                slots_changed.push(idx);

                // Update cursor
                cursor_item.0.count += take;
                needed -= take;

                if needed <= 0 {
                    break;
                }
            }
        }
    }
}

fn handle_shift_click(
    packet: ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    open_inv_size: usize,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    // case 1: clicking in open inventory
    // when shift clicking, it moves the slot clicked to the last empty slot in the player's hotbar.
    // if the hotbar is full then it moves it to the first empty slot in the player's inventory
    // if slot is empty, check when was the last time the slot was clicked
    // if its within 1 tick of the current tick, then move all items with the exact item and nbt as the
    // last stack clicked to the player's hotbar or inventory
    // case 2: clicking in player's inventory
    // if the clicked slot is happening in the player's inventory then just move all items with the exact item and nbt to the open inventory
    // if the slot is empty, then move the last stack clicked to the slot
    // same behavior as case 1
    if packet.slot_idx < 0 || packet.slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    let slot_idx = packet.slot_idx as usize;
    let source_slot = inventories_mut[slot_idx].clone();

    // Skip if source slot is empty
    if source_slot.stack.is_empty() || source_slot.readonly {
        return;
    }
    // if we shift click an armor piece, we should try to move it to the appropriate armor slot.
    // if not just move it to the top of the inventory
    if player_only {
        let item = source_slot.stack.item;
        let target_slot = match item {
            _ if item.is_helmet() && inventories_mut[5].stack.is_empty() => Some(5),
            _ if item.is_chestplate() && inventories_mut[6].stack.is_empty() => Some(6),
            _ if item.is_leggings() && inventories_mut[7].stack.is_empty() => Some(7),
            _ if item.is_boots() && inventories_mut[8].stack.is_empty() => Some(8),
            _ => None,
        };

        if let Some(target_idx) = target_slot {
            if inventories_mut[target_idx].readonly {
                return;
            }

            inventories_mut[target_idx].stack = source_slot.stack.clone();
            inventories_mut[target_idx].changed = true;
            slots_changed.push(target_idx);
            inventories_mut[slot_idx].stack = ItemStack::EMPTY;
            inventories_mut[slot_idx].changed = true;
            slots_changed.push(slot_idx);
            return;
        }
    }

    // Clear source slot immediately
    inventories_mut[slot_idx].stack = ItemStack::EMPTY;
    inventories_mut[slot_idx].changed = true;
    slots_changed.push(slot_idx);

    let mut to_move = source_slot.stack.clone();

    // Case 1: Clicking in open inventory
    if slot_idx < open_inv_size {
        // Try hotbar first (36-44)
        for target_idx in (open_inv_size + 27..=open_inv_size + 35).rev() {
            if try_move_to_slot(&mut to_move, &mut inventories_mut[target_idx]) {
                slots_changed.push(target_idx);
                if to_move.is_empty() {
                    break;
                }
            }
        }

        // Then try main inventory (9-35)
        if !to_move.is_empty() {
            for target_idx in open_inv_size..open_inv_size + 27 {
                if try_move_to_slot(&mut to_move, &mut inventories_mut[target_idx]) {
                    slots_changed.push(target_idx);
                    if to_move.is_empty() {
                        break;
                    }
                }
            }
        }
    } else {
        // Case 2: Clicking in player inventory
        for target_idx in 0..open_inv_size {
            if try_move_to_slot(&mut to_move, &mut inventories_mut[target_idx]) {
                slots_changed.push(target_idx);
                if to_move.is_empty() {
                    break;
                }
            }
        }
    }

    // If we couldn't move everything, put remainder back
    if !to_move.is_empty() {
        inventories_mut[slot_idx].stack = to_move;
    }
}

fn handle_hotbar_swap(
    packet: ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    open_inv_size: usize,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    // the client is pressing on numbers 1-9 or their hotbar binds
    // we just need to swap the two index provided by the packet in
    // slot_changes

    if packet.slot_idx < 0 || packet.slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    let slot_idx = packet.slot_idx as usize;
    let slot = inventories_mut[slot_idx].clone();

    // button 0 is the first slot in the hotbar of the player's inventory
    // button 8 is the last slot in the hotbar of the player's inventory
    let hotbar_idx = if player_only {
        if packet.button == 40 {
            // This is the offhand slot
            45
        } else {
            (packet.button as usize) + 36
        }
    } else {
        (packet.button as usize) + open_inv_size + 27
    };

    if hotbar_idx >= (inventories_mut.len() as usize) {
        return;
    }

    let hotbar_slot = inventories_mut[hotbar_idx].clone();

    if hotbar_slot.readonly || slot.readonly {
        return;
    }

    if player_only {
        if (5..=8).contains(&slot_idx) && !hotbar_slot.stack.is_empty() {
            let is_valid = match slot_idx {
                5 => hotbar_slot.stack.item.is_helmet(),
                6 => hotbar_slot.stack.item.is_chestplate(),
                7 => hotbar_slot.stack.item.is_leggings(),
                8 => hotbar_slot.stack.item.is_boots(),
                _ => false,
            };

            if !is_valid {
                return;
            }
        }
    }

    inventories_mut[slot_idx].stack = hotbar_slot.stack.clone();
    inventories_mut[hotbar_idx].stack = slot.stack.clone();
    inventories_mut[slot_idx].changed = true;
    inventories_mut[hotbar_idx].changed = true;

    slots_changed.push(slot_idx);
    slots_changed.push(hotbar_idx);
}

fn handle_drop_key(
    packet: ClickSlotC2s<'_>,
    query: &mut PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    cursor_item: &mut CursorItem,
    slots_changed: &mut Vec<usize>,
    player_only: bool
) {
    // if the button is 0, then drop 1 item from the slot_idx
    // if button is 1, then drop the entire stack from the slot_idx

    let slot_idx = packet.slot_idx;
    // if the slot_idx is -999, then drop whatever is in the cursor item
    if slot_idx == -999 {
        if cursor_item.0.is_empty() {
            return;
        }

        let mut dropped = cursor_item.0.clone();
        let mut dropped_count = 0;

        if packet.button == 0 {
            dropped_count = 1;
        } else if packet.button == 1 {
            dropped_count = dropped.count;
        }

        dropped.count = dropped_count;
        cursor_item.0.count -= dropped_count;

        if cursor_item.0.count == 0 {
            cursor_item.0 = ItemStack::EMPTY;
        }

        let event = event::DropItemStackEvent {
            client: query.id,
            from_slot: None,
            item: dropped,
        };

        query.events.push(event, query.world);
        return;
    }

    if slot_idx < 0 || slot_idx >= (inventories_mut.len() as i16) {
        return;
    }

    let slot = &mut inventories_mut[slot_idx as usize];

    if slot.stack.is_empty() || slot.readonly {
        return;
    }

    let mut dropped = slot.stack.clone();
    let mut dropped_count = 0;

    if packet.button == 0 {
        dropped_count = 1;
    } else if packet.button == 1 {
        dropped_count = dropped.count;
    }

    dropped.count = dropped_count;
    slot.stack.count -= dropped_count;

    if slot.stack.count == 0 {
        slot.stack = ItemStack::EMPTY;
    }

    slot.changed = true;
    slots_changed.push(slot_idx as usize);

    let event = event::DropItemStackEvent {
        client: query.id,
        from_slot: Some(slot_idx as i16),
        item: dropped,
    };

    query.events.push(event, query.world);
}

fn try_move_to_slot(source: &mut ItemStack, target: &mut ItemSlot) -> bool {
    // Try to stack with existing items
    if
        !target.stack.is_empty() &&
        target.stack.item == source.item &&
        target.stack.nbt == source.nbt &&
        !target.readonly
    {
        let available_space = target.stack.item.max_stack() - target.stack.count;
        let to_move = source.count.min(available_space);

        if to_move > 0 {
            target.stack.count += to_move;
            source.count -= to_move;
            target.changed = true;

            if source.count == 0 {
                *source = ItemStack::EMPTY;
            }
            return true;
        }
    } else if
        // Try empty slot
        target.stack.is_empty()
    {
        target.stack = source.clone();
        target.changed = true;
        *source = ItemStack::EMPTY;
        return true;
    }

    false
}

fn resync_inventory(
    compose: &Compose,
    system: &EntityView<'_>,
    inventory: &Vec<ItemSlot>,
    inv_state: &InventoryState,
    cursor_item: &CursorItem,
    stream_id: ConnectionId
) {
    let packet = &(play::InventoryS2c {
        window_id: inv_state.window_id(),
        state_id: VarInt(inv_state.state_id()),
        slots: Cow::Owned(
            inventory
                .into_iter()
                .map(|slot| slot.stack.clone())
                .collect()
        ),
        carried_item: Cow::Borrowed(&cursor_item.0),
    });

    compose.unicast(packet, stream_id, *system).unwrap();

    let packet = &(play::ScreenHandlerSlotUpdateS2c {
        window_id: -1,
        state_id: VarInt(inv_state.state_id()),
        slot_idx: -1,
        slot_data: Cow::Borrowed(&cursor_item.0),
    });

    compose.unicast(packet, stream_id, *system).unwrap();
}

pub fn close_handled_screen(
    mut data: &'static [u8],
    query: &PacketSwitchQuery<'_>
) -> anyhow::Result<()> {
    let pkt = play::CloseHandledScreenC2s::decode(&mut data)?;

    // try to remove OpenInventory from the player if pkt.window_id isnt 0
    if pkt.window_id != 0 {
        query.id.entity_view(query.world).remove::<OpenInventory>();
    }

    Ok(())
}
