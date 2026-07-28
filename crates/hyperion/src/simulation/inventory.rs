//! Containers, the player inventory, and the packets that keep a client's copy
//! of them in step with the server's.
//!
//! # Two item models meet here
//!
//! The simulation still holds items as valence's 1.20.1 [`ItemStack`]: an
//! [`ItemKind`], a count, and a blob of NBT. Protocol 776 has neither of the
//! last two in that form -- an item is a count, a *26.2* registry id, and a
//! patch over the component defaults its type implies. So everything leaving
//! this module goes through [`slot_of`], and nothing else in the file builds a
//! wire item by hand.
//!
//! The translation is not total, and the gaps are listed on
//! [`components_of`]. What matters for correctness is that a gap drops a
//! decoration rather than shifting a byte: [`Slot`] is encoded by the proto
//! crate's own codec, so an item this file understands badly still occupies
//! exactly the bytes the client expects to read.

use std::{borrow::Cow, sync::LazyLock};

use flecs_ecs::{
    core::{EntityViewGet, SystemAPI, World, flecs, id},
    macros::{Component, observer, system},
    prelude::Module,
};
use hyperion_inventory::{
    CursorItem, Inventory, InventoryState, ItemKindExt, ItemSlot, OpenInventory, PlayerInventory,
};
use hyperion_minecraft_proto::{
    Encode, RegistryId, Writer,
    generated::{packet_id::play::clientbound::PacketId, registry},
    item::{
        DataComponentPatch, ItemStack as WireItemStack, Slot,
        payload::{CustomName, DyedColor, EnchantmentGlintOverride, Payload, Text},
    },
    nbt,
    packets::play::{
        clientbound::{ContainerClose, OpenScreen},
        inventory::{
            ContainerSetContent, ContainerSetSlot, EquipmentSlot, SetCursorItem, SetEquipment,
        },
    },
    text::Component as TextComponent,
};
use hyperion_utils::EntityExt;
use tracing::{debug, error};
use valence_protocol::{
    ItemKind, VarInt,
    nbt::{Compound, Value},
    packets::play::{
        ClickSlotC2s, UpdateSelectedSlotC2s,
        click_slot_c2s::{ClickMode, SlotChange},
        open_screen_s2c::WindowType,
    },
};
use valence_server::ItemStack;

use super::{Player, event, handlers::PacketSwitchQuery};
use crate::net::{
    Compose, ConnectionId, DataBundle,
    protocol::{Clientbound, send},
};

#[derive(Component)]
pub struct InventoryModule;

impl Module for InventoryModule {
    fn module(world: &World) {
        world.component::<OpenInventory>();
        world.component::<InventoryState>();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, InventoryState)>();

        observer!(
            world,
            flecs::OnSet,
            &OpenInventory,
            &Compose,
            &mut InventoryState,
            &CursorItem,
            &ConnectionId,
        )
        .each_iter(
            |it, _row, (open_inventory, compose, inv_state, cursor_item, io)| {
                let world = it.world();
                let stream_id = *io;

                inv_state.set_window_id();

                open_inventory
                    .entity
                    .entity_view(world)
                    .try_get::<&mut Inventory>(|inventory| {
                        let Some(menu) = menu_id(inventory.kind()) else {
                            error!(
                                "no minecraft:menu entry for {:?}; not opening the screen",
                                inventory.kind()
                            );
                            return;
                        };

                        // The title is a text component, which since 1.20.5 is
                        // NBT on the wire rather than JSON. A plain string is
                        // a legal component and is what `to_tag` collapses an
                        // unstyled literal to.
                        let title = TextComponent::text(inventory.title());
                        let opened = OpenScreen {
                            container_id: i32::from(inv_state.window_id()),
                            r#type: RegistryId(menu),
                            title: title.to_tag(),
                        };

                        let content = ContainerSetContent {
                            container_id: i32::from(inv_state.window_id()),
                            state_id: inv_state.state_id(),
                            items: inventory
                                .slots()
                                .iter()
                                .map(|s| slot_of(&s.stack))
                                .collect(),
                            carried_item: slot_of(&cursor_item.0),
                        };

                        if let Err(error) =
                            send(compose, stream_id, PacketId::OpenScreen.to_raw(), &opened)
                                .and_then(|()| {
                                    send(
                                        compose,
                                        stream_id,
                                        PacketId::ContainerSetContent.to_raw(),
                                        &content,
                                    )
                                })
                        {
                            error!("could not open the container screen: {error}");
                        }
                    })
                    .expect("open inventory: no inventory found");
            },
        );

        observer!(
            world,
            flecs::OnRemove,
            &OpenInventory,
            &Compose,
            &mut InventoryState,
            &ConnectionId,
        )
        .each_iter(|_it, _row, (_open_inventory, compose, inv_state, io)| {
            let stream_id = *io;
            let packet = ContainerClose(i32::from(inv_state.window_id()));

            inv_state.reset_window_id();

            if let Err(error) = send(
                compose,
                stream_id,
                PacketId::ContainerClose.to_raw(),
                &packet,
            ) {
                error!("could not close the container screen: {error}");
            }
        });

        system!(
            "update_player_inventory",
            world,
            &Compose,
            &mut PlayerInventory,
            &InventoryState,
            &CursorItem,
            ?&OpenInventory,
            &ConnectionId,
        )
        .kind(id::<flecs_ecs::prelude::flecs::pipeline::OnStore>())
        .each_iter(
            |it, row, (compose, inventory, inv_state, cursor_item, open_inventory, io)| {
                let world = it.world();
                let entity = it.entity(row);
                let stream_id = *io;

                // What everyone else sees on this player: held item, offhand
                // and armour. The wearer is excluded because their own copy is
                // already right; the slot packets below are what correct it.
                let mut equipment: Vec<(EquipmentSlot, Slot<'_>)> = Vec::new();
                let hand_slot = inventory.held_slot();
                for (idx, slot) in inventory.slots_mut().iter_mut().enumerate() {
                    if !slot.changed {
                        continue;
                    }
                    if idx == usize::from(hand_slot) {
                        equipment.push((EquipmentSlot::Mainhand, slot_of(&slot.stack)));
                    }
                    if let Some(worn) = worn_slot(idx) {
                        equipment.push((worn, slot_of(&slot.stack)));
                    }
                }

                // `SetEquipment` has no length prefix and reads until a byte
                // without the continue bit, so an entry-less one would make the
                // client read the next packet's bytes as a slot.
                if !equipment.is_empty() {
                    let packet = SetEquipment {
                        entity: entity.minecraft_id(),
                        slots: equipment,
                    };
                    let bundle = Clientbound::new(PacketId::SetEquipment.to_raw(), &packet);

                    if let Err(error) = compose
                        .broadcast_channel(bundle, entity.into())
                        .exclude(stream_id)
                        .send()
                    {
                        error!("could not broadcast equipment: {error}");
                    }
                }

                if let Some(open_inventory) = open_inventory {
                    open_inventory
                        .entity
                        .entity_view(world)
                        .get::<&mut Inventory>(|open_inv| {
                            update_player_inventory_inner(
                                compose,
                                stream_id,
                                inv_state,
                                cursor_item,
                                open_inv
                                    .slots_mut()
                                    .iter_mut()
                                    .chain(inventory.slots_inventory_mut().iter_mut()),
                            );
                        });
                } else {
                    update_player_inventory_inner(
                        compose,
                        stream_id,
                        inv_state,
                        cursor_item,
                        inventory.slots_mut().iter_mut(),
                    );
                }
            },
        );
    }
}

fn update_player_inventory_inner<'a>(
    compose: &Compose,
    stream_id: ConnectionId,
    inv_state: &InventoryState,
    cursor_item: &CursorItem,
    inventories_mut: impl Iterator<Item = &'a mut ItemSlot>,
) {
    let mut bundle = DataBundle::new(compose);
    let mut changed_slots = false;
    let container_id = i32::from(inv_state.window_id());
    for (idx, slot) in inventories_mut.enumerate() {
        if !slot.changed {
            continue;
        }
        let Ok(index) = i16::try_from(idx) else {
            error!("slot {idx} is past what a container can address");
            continue;
        };
        let packet = ContainerSetSlot {
            container_id,
            state_id: inv_state.state_id(),
            slot: index,
            item_stack: slot_of(&slot.stack),
        };

        if let Err(error) = bundle.add_packet(Clientbound::new(
            PacketId::ContainerSetSlot.to_raw(),
            &packet,
        )) {
            error!("could not encode a slot update: {error}");
            continue;
        }
        slot.changed = false;
        changed_slots = true;
    }

    if changed_slots {
        if let Err(error) = bundle.unicast(stream_id) {
            error!("could not send slot updates: {error}");
        }
        send_cursor(compose, stream_id, cursor_item);
    }
}

/// Tell the client what the cursor is holding.
///
/// Before 1.21.2 this was a [`ContainerSetSlot`] addressed to window `-1`,
/// slot `-1`. That is now an ordinary slot update for a window that does not
/// exist, so it is dropped rather than applied and the dragged stack stops
/// updating; the dedicated packet is the only spelling left.
fn send_cursor(compose: &Compose, stream_id: ConnectionId, cursor_item: &CursorItem) {
    let packet = SetCursorItem {
        contents: slot_of(&cursor_item.0),
    };
    if let Err(error) = send(
        compose,
        stream_id,
        PacketId::SetCursorItem.to_raw(),
        &packet,
    ) {
        error!("could not send the cursor item: {error}");
    }
}

pub fn handle_update_selected_slot(
    packet: UpdateSelectedSlotC2s,
    query: &mut PacketSwitchQuery<'_>,
) {
    let Ok(slot) = u8::try_from(packet.slot) else {
        return;
    };

    if query.inventory.set_cursor(u16::from(slot)).is_err() {
        return;
    }

    let event = event::UpdateSelectedSlotEvent {
        client: query.id,
        slot,
    };

    query.events.push(event, query.world);
}

#[expect(clippy::too_many_arguments)]
fn handle_click_slot_inner<'a>(
    packet: &ClickSlotC2s<'_>,
    query: &PacketSwitchQuery<'_>,
    inv_state: &mut InventoryState,
    player_inventory: &'a mut PlayerInventory,
    cursor_item: &mut CursorItem,
    readonly: bool,
    open_inv_size: usize,
    player_only: bool,
    mut inventories_mut: Vec<&'a mut ItemSlot>,
) {
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
            &inventories_mut,
            inv_state,
            cursor_item,
            query.io_ref,
        );
        return;
    }

    if packet.state_id != VarInt(inv_state.state_id()) {
        resync_inventory(
            query.compose,
            &inventories_mut,
            inv_state,
            cursor_item,
            query.io_ref,
        );
    }

    if readonly {
        resync_inventory(
            query.compose,
            &inventories_mut,
            inv_state,
            cursor_item,
            query.io_ref,
        );

        return;
    }
    // button 0 is left click
    // button 1 is right click
    // button 2 is middle click

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
            match packet.button {
                0 => {
                    handle_left_click_slot(
                        packet,
                        query,
                        &mut inventories_mut,
                        inv_state,
                        cursor_item,
                        player_only,
                    );
                }
                1 => {
                    handle_right_click_slot(
                        packet,
                        query,
                        &mut inventories_mut,
                        cursor_item,
                        player_only,
                    );
                }
                // nothing implemented for middle click yet
                _ => {}
            }
        }
        ClickMode::Drag => {
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
                    handle_left_drag_slot(&mut cursor, &slots, &mut inventories_mut, player_only);
                }
                6 => {
                    handle_right_drag_slot(&mut cursor, &slots, &mut inventories_mut, player_only);
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
                player_only,
            );
        }
        ClickMode::ShiftClick => {
            handle_shift_click(packet, &mut inventories_mut, open_inv_size, player_only);
        }
        ClickMode::Hotbar => {
            handle_hotbar_swap(packet, &mut inventories_mut, open_inv_size, player_only);
        }
        ClickMode::CreativeMiddleClick => {}
        ClickMode::DropKey => {
            handle_drop_key(
                packet,
                query,
                &mut inventories_mut,
                cursor_item,
                player_only,
            );
        }
    }

    resync_inventory(
        query.compose,
        &inventories_mut,
        inv_state,
        cursor_item,
        query.io_ref,
    );

    let mut has_changed = false;
    for slot in &inventories_mut {
        if slot.changed {
            has_changed = true;
            break;
        }
    }

    if has_changed {
        inv_state.set_last_button(0, query.compose.global().tick);
        inv_state.set_last_mode(ClickMode::Click, query.compose.global().tick);
    }
}

pub fn handle_click_slot(packet: &ClickSlotC2s<'_>, query: &mut PacketSwitchQuery<'_>) {
    // In here we need to handle different behaviors based on the click mode
    // First of we need to check if the player has the inventory open
    // Then we need to check if that inventory is readonly
    // If so then we need to resync the inventory with the client to make sure the client is in sync with the server
    query.id.entity_view(query.world).get::<(
        &mut InventoryState,
        Option<&OpenInventory>,
        &mut PlayerInventory,
        &mut CursorItem,
    )>(
        |(inv_state, open_inventory, player_inventory, cursor_item)| {
            if let Some(open_inventory) = open_inventory {
                open_inventory
                    .entity
                    .entity_view(query.world)
                    .get::<&mut Inventory>(|open_inv| {
                        let readonly = open_inv.readonly();
                        let open_inv_size = open_inv.size();
                        let player_only = false;

                        let inventories_mut: Vec<&mut ItemSlot> =
                            open_inv.slots_mut().iter_mut().collect();

                        handle_click_slot_inner(
                            packet,
                            query,
                            inv_state,
                            player_inventory,
                            cursor_item,
                            readonly,
                            open_inv_size,
                            player_only,
                            inventories_mut,
                        );
                    });
            } else {
                let readonly = player_inventory.readonly();
                let open_inv_size = 0;
                let player_only = true;
                let inventories_mut: Vec<&mut ItemSlot> = vec![];

                handle_click_slot_inner(
                    packet,
                    query,
                    inv_state,
                    player_inventory,
                    cursor_item,
                    readonly,
                    open_inv_size,
                    player_only,
                    inventories_mut,
                );
            }
        },
    );
}

fn handle_left_click_slot(
    packet: &ClickSlotC2s<'_>,
    query: &PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    inv_state: &mut InventoryState,
    cursor_item: &mut CursorItem,
    player_only: bool,
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

    let Ok(slot_idx) = usize::try_from(packet.slot_idx) else {
        return;
    };
    let Some(slot) = inventories_mut.get_mut(slot_idx) else {
        return;
    };

    if player_only && !cursor_item.0.is_empty() {
        let is_valid = match slot_idx {
            5 => cursor_item.0.item.is_helmet(),
            6 => cursor_item.0.item.is_chestplate(),
            7 => cursor_item.0.item.is_leggings(),
            8 => cursor_item.0.item.is_boots(),
            _ => true,
        };

        if !is_valid {
            return;
        }
    }
    if slot.readonly {
        return;
    }
    let cursor = cursor_item.0.clone();

    if slot.stack.is_empty() {
        slot.stack = cursor;
        slot.changed = true;
        cursor_item.0 = ItemStack::EMPTY;
        inv_state.set_last_stack_clicked(ItemStack::EMPTY, query.compose.global().tick);
    } else if slot.stack.item == cursor.item {
        let count = slot.stack.count.saturating_add(cursor.count);
        let max = slot.stack.item.max_stack();

        if count > max {
            let diff = count - max;
            slot.stack = ItemStack::new(cursor.item, max, cursor.nbt.clone());
            cursor_item.0 = ItemStack::new(cursor.item, diff, cursor.nbt);
        } else {
            slot.stack = ItemStack::new(cursor.item, count, cursor.nbt);
            cursor_item.0 = ItemStack::EMPTY;
        }

        slot.changed = true;
        inv_state.set_last_stack_clicked(slot.stack.clone(), query.compose.global().tick);
    } else {
        let old_slot_stack = slot.stack.clone();
        slot.stack = cursor;
        slot.changed = true;
        cursor_item.0 = old_slot_stack.clone();
        inv_state.set_last_stack_clicked(old_slot_stack, query.compose.global().tick);
    }
}

fn handle_right_click_slot(
    packet: &ClickSlotC2s<'_>,
    query: &PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    cursor_item: &mut CursorItem,
    player_only: bool,
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
                query.world,
            );
        }
        return;
    }

    let Ok(slot_idx) = usize::try_from(packet.slot_idx) else {
        return;
    };
    let Some(slot) = inventories_mut.get_mut(slot_idx) else {
        return;
    };

    if player_only {
        let slot_idx = packet.slot_idx;
        if !cursor_item.0.is_empty() {
            let is_valid = match slot_idx {
                5 => cursor_item.0.item.is_helmet(),
                6 => cursor_item.0.item.is_chestplate(),
                7 => cursor_item.0.item.is_leggings(),
                8 => cursor_item.0.item.is_boots(),
                _ => true,
            };

            if !is_valid {
                return;
            }
        }
    }

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
    } else if slot.stack.is_empty() && !slot.readonly {
        slot.stack = ItemStack::new(cursor_item.0.item, 1, cursor_item.0.nbt.clone());
        cursor_item.0.count = cursor_item.0.count.saturating_sub(1);
        if cursor_item.0.count == 0 {
            cursor_item.0 = ItemStack::EMPTY;
        }
        changed = true;
    } else if slot.stack.item == cursor_item.0.item
        && slot.stack.nbt == cursor_item.0.nbt
        && slot.stack.count < slot.stack.item.max_stack()
        && !slot.readonly
    {
        slot.stack.count = slot.stack.count.saturating_add(1);
        cursor_item.0.count = cursor_item.0.count.saturating_sub(1);
        if cursor_item.0.count == 0 {
            cursor_item.0 = ItemStack::EMPTY;
        }
        changed = true;
    }

    if changed {
        slot.changed = true;
    }
}

fn handle_left_drag_slot(
    cursor: &mut ItemStack,
    slots: &[SlotChange],
    inventories_mut: &mut Vec<&mut ItemSlot>,
    player_only: bool,
) {
    let total = cursor.count;
    let Ok(slots_len) = i8::try_from(slots.len()) else {
        return;
    };

    let per_slot = total / slots_len;
    let mut remainder = total % slots_len;

    if player_only {
        let mut slots = slots.iter().map(|slot| slot.idx);
        if slots.any(|slot| (5..=8).contains(&slot)) {
            return;
        }
    }

    for slot_update in slots {
        let Ok(slot_idx) = usize::try_from(slot_update.idx) else {
            return;
        };
        let Some(slot) = inventories_mut.get_mut(slot_idx) else {
            continue;
        };
        let mut stack = slot.stack.clone();

        if slot.readonly {
            continue;
        }

        // If the slot is empty, set both item and nbt, then count
        if stack.is_empty() {
            let available_space = cursor.item.max_stack();
            let to_add = per_slot.min(available_space);
            if to_add > 0 {
                stack.item = cursor.item;
                stack.nbt.clone_from(&cursor.nbt);
                stack.count = to_add;
            }
            // Track remainder if not all per_slot could fit
            remainder = remainder.saturating_add(per_slot - to_add);
        } else if
        // If the slot is not empty but matches cursor item + nbt
        stack.item == cursor.item && stack.nbt == cursor.nbt {
            let available_space = stack.item.max_stack() - stack.count;
            let to_add = per_slot.min(available_space);
            stack.count = stack.count.saturating_add(to_add);
            // Track remainder if not all per_slot could fit
            remainder = remainder.saturating_add(per_slot - to_add);
        }

        // Update the slot and mark it changed if any addition happened
        if stack != slot.stack && !slot.readonly {
            slot.stack = stack;
            slot.changed = true;
        }
    }
    // Update cursor to leftover remainder
    cursor.count = remainder;
}

fn handle_right_drag_slot(
    cursor: &mut ItemStack,
    slots: &[SlotChange],
    inventories_mut: &mut Vec<&mut ItemSlot>,
    player_only: bool,
) {
    if player_only {
        let mut slots = slots.iter().map(|slot| slot.idx);
        if slots.any(|slot| (5..=8).contains(&slot)) {
            return;
        }
    }

    for slot_update in slots {
        if cursor.count == 0 {
            break;
        }

        let Ok(slot_idx) = usize::try_from(slot_update.idx) else {
            return;
        };
        let Some(slot) = inventories_mut.get_mut(slot_idx) else {
            continue;
        };
        let mut stack = slot.stack.clone();

        if slot.readonly {
            continue;
        }

        if stack.is_empty() {
            stack.item = cursor.item;
            stack.nbt.clone_from(&cursor.nbt);
            stack.count = 1;
            cursor.count -= 1;
        } else if
        // If the slot is not empty but matches cursor item + nbt
        stack.item == cursor.item && stack.nbt == cursor.nbt {
            let available_space = stack.item.max_stack() - stack.count;
            let to_add = (1).min(available_space);
            stack.count = stack.count.saturating_add(to_add);
            cursor.count -= to_add;
        }

        // Update the slot and mark it changed if any addition happened
        if stack != slot.stack && !slot.readonly {
            slot.stack = stack;
            slot.changed = true;
        }
    }
}

fn handle_double_click(
    packet: &ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    inv_state: &InventoryState,
    cursor_item: &mut CursorItem,
    _player_only: bool,
) {
    // if the slot is empty... check if the last stack clicked is the same as the cursor item
    // ignoring the count
    // and also see if the last tick was within 1 tick of the current tick
    // if so, then try to take any matching items from the cursor item and add it to the
    // count of cursor item till it reaches 64 or there are no more matching items
    // make sure the slot is empty as well

    let Ok(slot_idx) = usize::try_from(packet.slot_idx) else {
        return;
    };
    let Some(slot) = inventories_mut.get(slot_idx) else {
        return;
    };
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
                let slot = &mut *inventories_mut[idx];
                if slot.readonly {
                    continue;
                }
                slot.stack.count -= take;
                if slot.stack.count == 0 {
                    slot.stack = ItemStack::EMPTY;
                }
                slot.changed = true;

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
    packet: &ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    open_inv_size: usize,
    player_only: bool,
) {
    // case 1: clicking in open inventory
    // when shift clicking, it moves the slot clicked to the last empty slot in the player's hotbar.
    // if the hotbar is full then it moves it to the first empty slot in the player's inventory
    // if slot is empty, check when was the last time the slot was clicked
    // if its within 1 tick of the current tick, then move all items with the exact item and nbt as the
    // last stack clicked to the player's hotbar or inventory
    // case 2: clicking in player's inventory
    // The client sends a packet for each index they want to shift click
    let Ok(slot_idx) = usize::try_from(packet.slot_idx) else {
        return;
    };
    let Some(source_slot) = inventories_mut.get(slot_idx) else {
        return;
    };

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
            let Ok([source_slot, target_slot]) =
                inventories_mut.get_disjoint_mut([slot_idx, target_idx])
            else {
                return;
            };
            if target_slot.readonly {
                return;
            }

            target_slot.stack = std::mem::replace(&mut source_slot.stack, ItemStack::EMPTY);
            target_slot.changed = true;
            source_slot.changed = true;
            return;
        }
    }

    // Clear source slot immediately
    let source_slot = &mut *inventories_mut[slot_idx];
    let mut to_move = std::mem::replace(&mut source_slot.stack, ItemStack::EMPTY);
    source_slot.changed = true;

    // Case 1: Clicking in open inventory
    if slot_idx < open_inv_size {
        // Try hotbar first (36-44)
        for target_idx in (open_inv_size + 27..=open_inv_size + 35).rev() {
            if try_move_to_slot(&mut to_move, inventories_mut[target_idx]) && to_move.is_empty() {
                break;
            }
        }

        // Then try main inventory (9-35)
        if !to_move.is_empty() {
            for slot in inventories_mut.iter_mut().skip(open_inv_size).take(27) {
                if try_move_to_slot(&mut to_move, slot) && to_move.is_empty() {
                    break;
                }
            }
        }
    } else {
        // Case 2: Clicking in player inventory
        for slot in inventories_mut.iter_mut().take(open_inv_size) {
            if try_move_to_slot(&mut to_move, slot) && to_move.is_empty() {
                break;
            }
        }
    }

    // If we couldn't move everything, put remainder back
    if !to_move.is_empty() {
        inventories_mut[slot_idx].stack = to_move;
    }
}

fn handle_hotbar_swap(
    packet: &ClickSlotC2s<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    open_inv_size: usize,
    player_only: bool,
) {
    // the client is pressing on numbers 1-9 or their hotbar binds
    // we just need to swap the two index provided by the packet in
    // slot_changes

    // button 0 is the first slot in the hotbar of the player's inventory
    // button 8 is the last slot in the hotbar of the player's inventory
    let Ok(button) = usize::try_from(packet.button) else {
        return;
    };
    let hotbar_idx = if player_only {
        if packet.button == 40 {
            // This is the offhand slot
            45
        } else {
            button + 36
        }
    } else {
        button + open_inv_size + 27
    };

    let Ok(slot_idx) = usize::try_from(packet.slot_idx) else {
        return;
    };
    let Ok([slot, hotbar_slot]) = inventories_mut.get_disjoint_mut([slot_idx, hotbar_idx]) else {
        return;
    };

    if hotbar_slot.readonly || slot.readonly {
        return;
    }

    if player_only && !hotbar_slot.stack.is_empty() {
        let is_valid = match slot_idx {
            5 => hotbar_slot.stack.item.is_helmet(),
            6 => hotbar_slot.stack.item.is_chestplate(),
            7 => hotbar_slot.stack.item.is_leggings(),
            8 => hotbar_slot.stack.item.is_boots(),
            _ => true,
        };

        if !is_valid {
            return;
        }
    }

    std::mem::swap(&mut slot.stack, &mut hotbar_slot.stack);
    slot.changed = true;
    hotbar_slot.changed = true;
}

fn handle_drop_key(
    packet: &ClickSlotC2s<'_>,
    query: &PacketSwitchQuery<'_>,
    inventories_mut: &mut Vec<&mut ItemSlot>,
    cursor_item: &mut CursorItem,
    _player_only: bool,
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

    let Ok(slot_idx_usize) = usize::try_from(slot_idx) else {
        return;
    };
    let Some(slot) = inventories_mut.get_mut(slot_idx_usize) else {
        return;
    };

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

    let event = event::DropItemStackEvent {
        client: query.id,
        from_slot: Some(slot_idx),
        item: dropped,
    };

    query.events.push(event, query.world);
}

fn try_move_to_slot(source: &mut ItemStack, target: &mut ItemSlot) -> bool {
    // Try to stack with existing items
    if !target.stack.is_empty()
        && target.stack.item == source.item
        && target.stack.nbt == source.nbt
        && !target.readonly
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
    target.stack.is_empty() {
        target.stack = source.clone();
        target.changed = true;
        *source = ItemStack::EMPTY;
        return true;
    }

    false
}

fn resync_inventory(
    compose: &Compose,
    inventories_mut: &[&mut ItemSlot],
    inv_state: &InventoryState,
    cursor_item: &CursorItem,
    stream_id: ConnectionId,
) {
    let packet = ContainerSetContent {
        container_id: i32::from(inv_state.window_id()),
        state_id: inv_state.state_id(),
        items: inventories_mut
            .iter()
            .map(|slot| slot_of(&slot.stack))
            .collect(),
        carried_item: slot_of(&cursor_item.0),
    };

    if let Err(error) = send(
        compose,
        stream_id,
        PacketId::ContainerSetContent.to_raw(),
        &packet,
    ) {
        error!("could not resynchronise the container: {error}");
    }

    send_cursor(compose, stream_id, cursor_item);
}

// --- the 1.20.1 item model, translated ------------------------------------

/// The 26.2 item registry id for each of valence's 1.20.1 [`ItemKind`]s.
///
/// Indexed by `ItemKind::to_raw`, which is the *1.20.1* id and has drifted a
/// long way: 26.2 has 1537 items to 1.20.1's 1255, and every insertion in
/// between shifted the ones after it. Names are the only stable handle, so the
/// table is built by name once rather than by trusting either numbering.
///
/// `-1` marks a name 26.2 no longer has, which [`item_id`] turns into an empty
/// slot rather than into whatever item happens to sit at some other id.
static ITEM_IDS: LazyLock<Box<[i32]>> = LazyLock::new(|| {
    let mut table = vec![-1; ItemKind::ALL.len()];
    for kind in ItemKind::ALL {
        // Filled by `to_raw` rather than by iteration order, so the table
        // stays right even if `ALL` is ever emitted in some other order.
        let name = renamed(kind.to_str());
        if let Some(id) = registry::ITEM
            .id_of(&format!("minecraft:{name}"))
            .and_then(|id| i32::try_from(id).ok())
        {
            table[usize::from(kind.to_raw())] = id;
        }
    }
    table.into_boxed_slice()
});

/// The 26.2 name for a 1.20.1 item name, where the two differ.
///
/// Three items were renamed between the versions and nothing else was dropped,
/// so this is the whole delta rather than a sample of it. Each is a rename in
/// the vanilla registry, checked against `generated::registry::ITEM`.
fn renamed(name: &str) -> &str {
    match name {
        // 1.20.3 split the grass block's plant off from `grass`.
        "grass" => "short_grass",
        // 1.21.9 qualified the chain by its metal.
        "chain" => "iron_chain",
        // 1.20.5 qualified the scute by its animal, for the armadillo's.
        "scute" => "turtle_scute",
        _ => name,
    }
}

/// The 26.2 registry id for a 1.20.1 item, or `None` when 26.2 has no such
/// item.
fn item_id(kind: ItemKind) -> Option<i32> {
    let id = *ITEM_IDS.get(usize::from(kind.to_raw()))?;
    (id >= 0).then_some(id)
}

/// The 26.2 `minecraft:menu` id for a 1.20.1 window type.
///
/// Ordinals cannot be reused: 26.2 inserted `crafter_3x3` at position 7, so
/// everything from `anvil` onwards sits one higher than valence's enum says
/// and a numeric cast would open the wrong screen.
fn menu_id(kind: WindowType) -> Option<i32> {
    let name = match kind {
        WindowType::Generic9x1 => "minecraft:generic_9x1",
        WindowType::Generic9x2 => "minecraft:generic_9x2",
        WindowType::Generic9x3 => "minecraft:generic_9x3",
        WindowType::Generic9x4 => "minecraft:generic_9x4",
        WindowType::Generic9x5 => "minecraft:generic_9x5",
        WindowType::Generic9x6 => "minecraft:generic_9x6",
        WindowType::Generic3x3 => "minecraft:generic_3x3",
        WindowType::Anvil => "minecraft:anvil",
        WindowType::Beacon => "minecraft:beacon",
        WindowType::BlastFurnace => "minecraft:blast_furnace",
        WindowType::BrewingStand => "minecraft:brewing_stand",
        WindowType::Crafting => "minecraft:crafting",
        WindowType::Enchantment => "minecraft:enchantment",
        WindowType::Furnace => "minecraft:furnace",
        WindowType::Grindstone => "minecraft:grindstone",
        WindowType::Hopper => "minecraft:hopper",
        WindowType::Lectern => "minecraft:lectern",
        WindowType::Loom => "minecraft:loom",
        WindowType::Merchant => "minecraft:merchant",
        WindowType::ShulkerBox => "minecraft:shulker_box",
        WindowType::Smithing => "minecraft:smithing",
        WindowType::Smoker => "minecraft:smoker",
        WindowType::Cartography => "minecraft:cartography_table",
        WindowType::Stonecutter => "minecraft:stonecutter",
    };
    registry::MENU
        .id_of(name)
        .and_then(|id| i32::try_from(id).ok())
}

/// Which equipment slot a player inventory index shows up in, if any.
///
/// The indices are the player container's, unchanged since 1.8. The held item
/// is not here because which index that is depends on the selected hotbar
/// slot, so its caller reads that instead of a constant.
const fn worn_slot(index: usize) -> Option<EquipmentSlot> {
    match index {
        5 => Some(EquipmentSlot::Head),
        6 => Some(EquipmentSlot::Chest),
        7 => Some(EquipmentSlot::Legs),
        8 => Some(EquipmentSlot::Feet),
        45 => Some(EquipmentSlot::Offhand),
        _ => None,
    }
}

/// One simulation item stack as protocol 776 sends it.
///
/// An item the 26.2 registry does not have becomes an empty slot. Substituting
/// a neighbouring id would put a plausible-looking wrong item in the slot,
/// which is worse to debug than a hole, and there is no id that means "unknown
/// item".
fn slot_of(stack: &ItemStack) -> Slot<'static> {
    if stack.is_empty() {
        return Slot::Empty;
    }
    let Some(item) = item_id(stack.item) else {
        error!(
            "26.2 has no item named {}; sending an empty slot",
            stack.item.to_str()
        );
        return Slot::Empty;
    };

    Slot::Occupied(WireItemStack {
        count: i32::from(stack.count),
        item,
        components: components_of(stack.nbt.as_ref()),
    })
}

/// The component patch that carries what a 1.20.1 item's NBT meant.
///
/// # What is translated
///
/// * `display.Name` becomes `minecraft:custom_name`. The 1.20.1 value is a
///   JSON text component and the 26.2 one is the same component as NBT, so
///   this is a structural transcription rather than a reinterpretation.
/// * `display.color` becomes `minecraft:dyed_color`, the same packed
///   `0xRRGGBB` int.
/// * A non-empty `Enchantments` list becomes
///   `minecraft:enchantment_glint_override`. hyperion only ever writes that
///   list to make an item shimmer, and the override is how 26.2 spells that;
///   an item that wanted real enchantments would need each id mapped through
///   `minecraft:enchantment`, which nothing here asks for.
///
/// # What is not
///
/// `AttributeModifiers`, the written-book keys and `display.Lore` have 26.2
/// components but need per-field translation nobody needs yet. `Handler` is
/// hyperion's own entity id for an item's click handler, is read back off the
/// server's copy of the stack, and deliberately does not reach the client.
/// Everything else is dropped and logged at debug.
fn components_of(source: Option<&Compound>) -> DataComponentPatch<'static> {
    let mut patch = DataComponentPatch::empty();
    let Some(source) = source else {
        return patch;
    };

    for (key, value) in source {
        match (key.as_str(), value) {
            ("display", Value::Compound(display)) => translate_display(display, &mut patch),
            ("Enchantments", Value::List(list)) if !list.is_empty() => {
                set_or_log(&mut patch, &EnchantmentGlintOverride(true));
            }
            ("Handler", _) => {}
            _ => debug!("no 26.2 component for item NBT key {key}"),
        }
    }

    patch
}

/// Fold the 1.20.1 `display` compound into the components that replaced it.
fn translate_display(display: &Compound, patch: &mut DataComponentPatch<'static>) {
    for (key, value) in display {
        match (key.as_str(), value) {
            ("Name", Value::String(json)) => match encoded_component(json) {
                Some(bytes) => set_or_log(patch, &CustomName(Text::from_bytes(&bytes))),
                None => debug!("item name is not a text component: {json}"),
            },
            ("color", Value::Int(color)) => set_or_log(patch, &DyedColor(*color)),
            _ => debug!("no 26.2 component for item display key {key}"),
        }
    }
}

/// A 1.20.1 JSON text component, as the network NBT tag 26.2 wants.
///
/// Since 1.20.5 `ComponentSerialization.STREAM_CODEC` writes a component as
/// NBT rather than as JSON, and the *shape* did not change with it: the same
/// keys, holding the same things. So this is a structural transcription and
/// not a reinterpretation, and it refuses the cases where the two formats do
/// not line up -- a JSON `null` has no tag, and NBT has no unsigned or
/// arbitrary-precision number -- rather than guessing.
fn encoded_component(json: &str) -> Option<Vec<u8>> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let mut writer = Writer::new();
    tag_of(&value)?.encode(&mut writer).ok()?;
    Some(writer.into_vec())
}

/// One JSON value as the NBT tag with the same meaning.
fn tag_of(value: &serde_json::Value) -> Option<nbt::Tag<'static>> {
    Some(match value {
        // NBT has no boolean; the server reads a byte for one either way.
        serde_json::Value::Bool(flag) => nbt::Tag::Byte(i8::from(*flag)),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(integer) => i32::try_from(integer).map_or(nbt::Tag::Long(integer), nbt::Tag::Int),
            None => nbt::Tag::Double(number.as_f64()?),
        },
        serde_json::Value::String(text) => nbt::Tag::String(Cow::Owned(text.clone())),
        // A mixed list is legal since 1.21.5, so `extra` holding both strings
        // and objects survives; `List` does the boxing that needs.
        serde_json::Value::Array(items) => nbt::Tag::List(
            items
                .iter()
                .map(tag_of)
                .collect::<Option<nbt::List<'_>>>()?,
        ),
        serde_json::Value::Object(fields) => {
            let mut compound = nbt::Compound::new();
            for (key, field) in fields {
                compound.insert(key.clone(), tag_of(field)?);
            }
            nbt::Tag::Compound(compound)
        }
        serde_json::Value::Null => return None,
    })
}

/// Write one component into the patch, logging rather than failing.
///
/// A payload that will not encode is a bug in this file, not in the item, and
/// dropping the decoration keeps the rest of the stack on the wire intact.
fn set_or_log<'a, P: Payload<'a>>(patch: &mut DataComponentPatch<'static>, payload: &P) {
    if let Err(error) = patch.set(payload) {
        error!("could not encode an item component: {error}");
    }
}

/// Closes whatever inventory the player currently has open. Removing [`OpenInventory`] is what
/// makes the close screen packet go out, via the `flecs::OnRemove` observer above.
pub fn handle_close_window(query: &PacketSwitchQuery<'_>) {
    query
        .id
        .entity_view(query.world)
        .remove(id::<OpenInventory>());
}

#[cfg(test)]
mod tests {
    use hyperion_minecraft_proto::{Encode, Writer, item::Slot};
    use valence_protocol::{
        ItemKind, ItemStack, nbt::Compound, packets::play::open_screen_s2c::WindowType,
    };

    use super::{ITEM_IDS, item_id, menu_id, registry, slot_of};

    fn encoded(slot: &Slot<'_>) -> Vec<u8> {
        let mut writer = Writer::new();
        slot.encode(&mut writer).expect("encode");
        writer.into_vec()
    }

    #[test]
    fn every_1_20_1_item_has_a_26_2_id() {
        // A hole here means a rename or a removal the `renamed` table does not
        // know about, and every one of those silently turns an item into an
        // empty slot. Naming them is the point: a count alone would not say
        // which.
        let missing: Vec<&str> = ItemKind::ALL
            .iter()
            .filter(|kind| item_id(**kind).is_none())
            .map(|kind| kind.to_str())
            .collect();
        assert!(missing.is_empty(), "no 26.2 id for {missing:?}");
        assert_eq!(ITEM_IDS.len(), ItemKind::ALL.len());
    }

    #[test]
    fn ids_are_looked_up_by_name_not_carried_over() {
        // Stone happens to be id 1 in both, so it proves nothing on its own.
        // A diamond sword is 797 in 1.20.1 and 964 in 26.2, which is exactly
        // what carrying the number over instead of the name would get wrong.
        assert_eq!(item_id(ItemKind::Stone), Some(1));
        assert_eq!(ItemKind::DiamondSword.to_raw(), 797);
        assert_eq!(item_id(ItemKind::DiamondSword), Some(964));
    }

    #[test]
    fn renamed_items_resolve_through_their_new_names() {
        // The three names 26.2 no longer has. Each resolves only because
        // `renamed` maps it, so this is what would catch that table being
        // dropped as "dead code".
        for (old, new) in [
            (ItemKind::Grass, "minecraft:short_grass"),
            (ItemKind::Chain, "minecraft:iron_chain"),
            (ItemKind::Scute, "minecraft:turtle_scute"),
        ] {
            let expected = registry::ITEM
                .id_of(new)
                .and_then(|id| i32::try_from(id).ok());
            assert_eq!(item_id(old), expected, "{} -> {new}", old.to_str());
        }
    }

    #[test]
    fn menu_ids_are_not_the_1_20_1_ordinals() {
        // 26.2 inserted crafter_3x3 at 7, so everything after generic_3x3 is
        // one higher than valence's enum position.
        assert_eq!(menu_id(WindowType::Generic9x1), Some(0));
        assert_eq!(menu_id(WindowType::Generic3x3), Some(6));
        assert_eq!(menu_id(WindowType::Anvil), Some(8));
        // And the one whose name also changed.
        assert_eq!(menu_id(WindowType::Cartography), Some(23));
    }

    #[test]
    fn an_empty_stack_is_one_zero_byte() {
        assert_eq!(encoded(&slot_of(&ItemStack::EMPTY)), [0]);
        // A count of zero is empty whatever the item says, matching
        // `ItemStack.isEmpty`.
        let zero = ItemStack::new(ItemKind::DiamondSword, 0, None);
        assert_eq!(encoded(&slot_of(&zero)), [0]);
    }

    #[test]
    fn a_plain_stack_carries_an_empty_patch() {
        let stack = ItemStack::new(ItemKind::DiamondSword, 1, None);
        // 01 count, c407 item 964, 00 00 nothing added and nothing removed.
        assert_eq!(encoded(&slot_of(&stack)), [0x01, 0xc4, 0x07, 0x00, 0x00]);
    }

    #[test]
    fn a_display_name_becomes_a_custom_name_component() {
        let mut display = Compound::new();
        display.insert("Name", r#"{"text":"Excalibur"}"#.to_owned());
        let mut nbt = Compound::new();
        nbt.insert("display", display);

        let stack = ItemStack::new(ItemKind::DiamondSword, 1, Some(nbt));
        let encoded = encoded(&slot_of(&stack));

        // 01 c407     one diamond sword
        // 01 00       one component added, none removed
        // 06          minecraft:custom_name
        // 0a          TAG_Compound, since the JSON is an object
        // 08 0004 74657874  "text"
        // 0009 457863616c69627572  "Excalibur"
        // 00          TAG_End
        let expected = b"\x01\xc4\x07\x01\x00\x06\x0a\x08\x00\x04text\x00\x09Excalibur\x00";
        assert_eq!(encoded, expected);
    }
}
