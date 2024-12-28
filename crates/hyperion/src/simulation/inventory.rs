use std::{ borrow::Cow, cell::Cell, ops::Deref };
use flecs_ecs::{
    core::{ flecs, EntityView, EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World },
    macros::{ observer, system, Component },
    prelude::Module,
};
use hyperion_inventory::{
    CursorItem,
    Inventory,
    InventoryState,
    ItemSlot,
    OpenInventory,
    PlayerInventory,
};
use hyperion_utils::EntityExt;
use serde::de;
use tracing::debug;
use valence_protocol::{
    packets::play::{
        self,
        click_slot_c2s::ClickMode,
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
use crate::{ net::{ Compose, ConnectionId, DataBundle }, storage };

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

        system!(
            "update_player_inventory",
            world,
            &Compose($),
            &mut PlayerInventory,
            &mut InventoryState,
            &CursorItem,
            ?&OpenInventory,
            &ConnectionId,
        )
            .multi_threaded()
            .kind::<flecs_ecs::prelude::flecs::pipeline::OnStore>()
            .each_iter(|it, row, (compose, inventory, inv_state, cursor, open_inventory, io)| {
                let system = it.system();
                let world = it.world();
                let entity = it.entity(row);
                let _entity_id = VarInt(entity.minecraft_id());
                let stream_id = *io;

                let mut changed_slots: Vec<(ItemSlot, usize)> = Vec::new();
                let (inventories, window_id) = if let Some(open_inventory) = open_inventory {
                    open_inventory.entity.entity_view(world).get::<&mut Inventory>(|open_inv| {
                        let changed = open_inv.has_changed() || inventory.has_changed();

                        // inventory has changed
                        if changed {
                            // group the player's inventory and the open inventory together
                            {
                                let mut all_slots = open_inv.slots_mut().to_vec();
                                all_slots.extend(inventory.slots_inventory_mut().to_vec());
                                // loop through all slots in the inventory
                                for (idx, slot) in all_slots.iter_mut().enumerate() {
                                    // if the slot has changed
                                    if slot.changed {
                                        // add the slot to the list of changed slots
                                        changed_slots.push((slot.clone(), idx));
                                        slot.changed = false;
                                    }
                                }
                            }

                            inv_state.increment_state_id();
                            open_inv.set_changed(0);
                        }
                        let mut inventories = open_inv.slots().clone();
                        inventories.extend(inventory.slots_inventory().iter().cloned());
                        (inventories, inv_state.window_id())
                    })
                } else {
                    let changed = inventory.has_changed();

                    // inventory has changed
                    if changed {
                        // group the player's inventory and the open inventory together
                        {
                            let mut all_slots = inventory.slots_mut().to_vec();
                            // loop through all slots in the inventory
                            for (idx, slot) in all_slots.iter_mut().enumerate() {
                                // if the slot has changed
                                if slot.changed {
                                    // add the slot to the list of changed slots
                                    changed_slots.push((slot.clone(), idx));
                                    slot.changed = false;
                                }
                            }
                        }

                        inv_state.increment_state_id();
                        inventory.set_changed(0);
                    }
                    let inventories = inventory.slots().clone();
                    (inventories, 0)
                };

                // if more than 10 slots have changed, send the entire inventory
                if changed_slots.len() > 10 {
                    let packet = &(play::InventoryS2c {
                        window_id,
                        state_id: VarInt(inv_state.state_id()),
                        slots: Cow::Owned(
                            inventories
                                .into_iter()
                                .map(|slot| slot.stack.clone())
                                .collect()
                        ),
                        carried_item: Cow::Borrowed(&cursor.0),
                    });

                    compose.unicast(packet, stream_id, system).unwrap();
                } else {
                    let mut bundle = DataBundle::new(compose, system);

                    for slot in changed_slots {
                        let packet = &(play::ScreenHandlerSlotUpdateS2c {
                            window_id: window_id as i8,
                            state_id: VarInt(inv_state.state_id()),
                            slot_idx: slot.1 as i16,
                            slot_data: Cow::Borrowed(&slot.0.stack),
                        });

                        bundle.add_packet(packet).unwrap();
                    }

                    bundle.unicast(stream_id).unwrap();
                }
            });
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

    // update the player's selected slot
    let hand = EquipmentEntry {
        slot: 0,
        item: query.inventory.get_cursor().stack.clone(),
    };

    // sync the player's selected slot with the client
    let packet = &(play::EntityEquipmentUpdateS2c {
        entity_id: VarInt(query.id.minecraft_id()),
        equipment: vec![hand],
    });

    query.compose.broadcast_local(packet, query.position.to_chunk(), query.system).send().unwrap();

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
            if let Some(open_inventory) = open_inventory {
                open_inventory.entity.entity_view(query.world).get::<&mut Inventory>(|open_inv| {
                    let readonly = open_inv.readonly();
                    let mut inventories_mut = open_inv
                        .slots_mut()
                        .iter_mut()
                        .chain(player_inventory.slots_inventory_mut())
                        .collect::<Vec<&mut ItemSlot>>();

                    // validate that packet_window_id is the same as the inv_state.window_id
                    if packet.window_id != inv_state.window_id() {
                        resync_inventory(
                            query.compose,
                            &query.system,
                            &inventories_mut,
                            inv_state,
                            cursor_item,
                            query.io_ref
                        );
                        return;
                    }

                    if readonly {
                        resync_inventory(
                            query.compose,
                            &query.system,
                            &inventories_mut,
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

                    // if the inventory is not readonly, handle the click
                    // in this case we need to check what button was clicked and what mode

                    // button 0 is left click
                    // button 1 is right click
                    // button 2 is middle click

                    // if the mode is click, and the button is 0, then check if its the same item as the cursor item
                    // if it is, check how many items are in the slot
                    // if its less than 64, then add as many items from the cursor item as possible till the count of the slot is 64
                    // if the slot is empty, then move the cursor item to the slot
                    // if its not the same item, then swap the cursor item with the slot item
                    // if the slot_idx is -999 that means the cursor item is being dropped

                    // if the mode is click, and the button is 1, then check if its the same item as the cursor item
                    // if it is the same or the slot is empty, then move 1 item from the cursor item to the slot

                    match packet.mode {
                        ClickMode::Click => {
                            let slot_idx = packet.slot_idx as u16;
                            match packet.button {
                                0 => {
                                    debug!("Cursor item: {:?}", cursor_item.0);
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
                                    let slot = inventories_mut[slot_idx as usize].clone();
                                    let cursor = cursor_item.0.clone();

                                    if slot.stack.is_empty() {
                                        inventories_mut[slot_idx as usize].stack = cursor;
                                        inventories_mut[slot_idx as usize].changed = true;
                                        cursor_item.0 = ItemStack::EMPTY;
                                    } else if slot.stack.item == cursor.item {
                                        let count = slot.stack.count + cursor.count;
                                        let max = slot.stack.item.max_stack();

                                        if count > max {
                                            let diff = count - max;
                                            inventories_mut[slot_idx as usize].stack =
                                                ItemStack::new(
                                                    cursor.item,
                                                    max,
                                                    cursor.nbt.clone()
                                                );
                                            inventories_mut[slot_idx as usize].changed = true;
                                            cursor_item.0 = ItemStack::new(
                                                cursor.item,
                                                diff,
                                                cursor.nbt.clone()
                                            );
                                        } else {
                                            inventories_mut[slot_idx as usize].stack =
                                                ItemStack::new(
                                                    cursor.item,
                                                    count,
                                                    cursor.nbt.clone()
                                                );
                                            inventories_mut[slot_idx as usize].changed = true;
                                            cursor_item.0 = ItemStack::EMPTY;
                                        }
                                    } else {
                                        let old_slot_stack = slot.stack.clone();
                                        inventories_mut[slot_idx as usize].stack = cursor;
                                        inventories_mut[slot_idx as usize].changed = true;
                                        cursor_item.0 = old_slot_stack;
                                    }
                                    open_inv.increment_slot(slot_idx as usize);
                                }
                                1 => {
                                    if packet.slot_idx == -999 {
                                        if cursor_item.0.is_empty() {
                                            return;
                                        }
                                        let new_stack = ItemStack::new(
                                            cursor_item.0.item,
                                            1,
                                            cursor_item.0.nbt.clone()
                                        );
                                        let event = event::DropItemStackEvent {
                                            client: query.id,
                                            from_slot: None,
                                            item: new_stack,
                                        };
                                        cursor_item.0.count -= 1;
                                        query.events.push(event, query.world);
                                        return;
                                    }

                                    let slot = inventories_mut[slot_idx as usize].clone();
                                    let cursor = cursor_item.0.clone();

                                    if slot.stack.is_empty() || slot.stack.item == cursor.item {
                                        // check count of slot
                                        let count = slot.stack.count + 1;
                                        let max = slot.stack.item.max_stack();
                                        if count > max {
                                            return;
                                        }

                                        inventories_mut[slot_idx as usize].stack = ItemStack::new(
                                            cursor.item,
                                            count,
                                            cursor.nbt.clone()
                                        );
                                        cursor_item.0.count -= 1;
                                    }
                                    open_inv.increment_slot(slot_idx as usize);
                                }
                                2 => {
                                    debug!("middle click");
                                }
                                _ => {}
                            }
                        }
                        ClickMode::ShiftClick => {
                            debug!("shift click");
                        }
                        ClickMode::Hotbar => {
                            debug!("hotbar");
                        }
                        ClickMode::CreativeMiddleClick => {
                            debug!("creative middle click");
                        }
                        ClickMode::DropKey => {
                            debug!("drop key");
                        }
                        ClickMode::Drag => {
                            debug!("drag");
                        }
                        ClickMode::DoubleClick => {
                            debug!("double click");
                        }
                    }
                });
            } else {
                debug!("no open inventory found");
            }

            // if open_inv is Some, check if the inventory is readonly

            /* let event = storage::ClickSlotEvent {
            window_id: inv_state.window_id(), 
            state_id: inv_state.state_id(),
            slot_idx: packet.slot_idx as u16,
            mode: packet.mode,
            button: packet.button,
            slot_changes: packet.slot_changes.to_vec(),
            carried_item: packet.carried_item,
        };

        debug!("click slot event from inventory.rs: {:?}", event);

        query.handlers.click.trigger_all(query, &event); */
        });
}

fn resync_inventory(
    compose: &Compose,
    system: &EntityView<'_>,
    inventory: &Vec<&mut ItemSlot>,
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
