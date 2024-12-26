use std::{borrow::Cow, str::FromStr};

use anyhow::{ensure, Context, Ok};
use flecs_ecs::{
    core::{World, flecs},
    macros::Component,
    prelude::{Module, *},
};
use hyperion_inventory::{CursorItem, Inventory, InventoryState, OpenInventory, PlayerInventory};
use tracing::{debug, error, warn};
use valence_protocol::{
    Decode, ItemKind, ItemStack, VarInt,
    packets::play::{
        self, ClickSlotC2s, UpdateSelectedSlotC2s, click_slot_c2s::ClickMode,
        open_screen_s2c::WindowType,
    },
};
use valence_text::Text;
use anyhow::bail;

use super::{event, handlers::PacketSwitchQuery, Player};
use crate::net::{Compose, ConnectionId};

pub struct InventoryWindow<'a> {
    player_inventory: &'a Inventory,
    open_inventory: Option<&'a Inventory>,
}

impl<'a> InventoryWindow<'a> {
    pub fn new(player_inventory: &'a Inventory, open_inventory: Option<&'a Inventory>) -> Self {
        Self {
            player_inventory,
            open_inventory,
        }
    }

    #[track_caller]
    pub fn slot(&self, idx: u16) -> &ItemStack {
        if let Some(open_inv) = self.open_inventory.as_ref() {
            if idx < open_inv.slots().len() as u16 {
                open_inv.get(idx).unwrap()
            } else {
                self.player_inventory
                    .get(convert_to_player_slot_id(open_inv, idx)).unwrap()
            }
        } else {
            self.player_inventory.get(idx).unwrap()
        }
    }

    #[track_caller]
    pub fn slot_count(&self) -> u16 {
        if let Some(open_inv) = &self.open_inventory {
            // when the window is split, we can only access the main slots of player's
            // inventory
            (9..=44).end() - (9..=44).start() + open_inv.slots().len() as u16
        } else {
            self.player_inventory.slots().len() as u16
        }
    }
}

#[derive(Component)]
pub struct InventoryModule;

impl Module for InventoryModule {
    fn module(world: &World) {
        world.component::<InventoryState>();
        world.component::<OpenInventory>();

        observer!(
            world,
            flecs::OnSet,
            &mut OpenInventory,
            &CursorItem,
            &Compose($),
        )
        .each_iter(|it, row, (open_inventory, cursor_item, compose)| {
            let world = it.world();
            let system = it.system();
            let entity = it.entity(row);

            entity
                .entity_view(world)
                .try_get::<(&mut PlayerInventory, &mut InventoryState, &ConnectionId)>(
                    |(player_inventory, inv_state, io)| {
                        let Some(inventory) = open_inventory
                            .entity
                            .entity_view(world)
                            .try_get::<&Inventory>(|inventory| inventory.clone())
                        else {
                            entity.remove::<OpenInventory>();

                            let packet = play::CloseScreenS2c {
                                window_id: inv_state.window_id,
                            };

                            compose.unicast(&packet, *io, system).unwrap();

                            return;
                        };

                        inv_state.window_id = inv_state.window_id % 100 + 1;
                        open_inventory.client_changed = 0;

                        let binding = Text::from_str(&inventory.title).unwrap();
                        let packet = play::OpenScreenS2c {
                            window_id: VarInt::from(i32::from(inv_state.window_id)),
                            window_type: WindowType::from(inventory.kind),
                            window_title: Cow::Borrowed(&binding),
                        };

                        compose.unicast(&packet, *io, system).unwrap();

                        let packet = play::InventoryS2c {
                            window_id: inv_state.window_id,
                            state_id: VarInt::from(inv_state.state_id.0),
                            slots: Cow::Borrowed(&inventory.slots()),
                            carried_item: Cow::Borrowed(&cursor_item.0),
                        };

                        compose.unicast(&packet, *io, system).unwrap();
                    },
                );
        });

        observer!(
            world,
            flecs::OnRemove,
            &OpenInventory,
            &Compose($),
        )
        .singleton()
        .each_iter(|it, row, (_open_inventory, compose)| {
            let world = it.world();
            let system = it.system();
            let entity = it.entity(row);

            entity
                .entity_view(world)
                .get::<(&InventoryState, &ConnectionId)>(|(inv_state, io)| {
                    let packet = play::CloseScreenS2c {
                        window_id: inv_state.window_id,
                    };

                    compose.unicast(&packet, *io, system).unwrap();
                });
        });

        system!(
            "update_open_inventories",
            world,
            &Compose($),
            &mut OpenInventory,
            &ConnectionId,
        )
        .singleton()
        .with::<flecs::pipeline::PostUpdate>()
        .each_iter(|it, row, (compose, open_inventory, io)| {
            let world = it.world();
            let system = it.system();
            let entity = it.entity(row);

            entity
                .entity_view(world)
                .get::<(&mut PlayerInventory, &mut InventoryState)>(
                    |(player_inventory, inv_state)| {
                        open_inventory
                            .entity
                            .entity_view(world)
                            .try_get::<&mut Inventory>(|inventory| {
                                if inventory.changed == u64::MAX {
                                    inv_state.state_id += 1;

                                    let packet = play::InventoryS2c {
                                        window_id: inv_state.window_id,
                                        state_id: VarInt::from(inv_state.state_id.0),
                                        slots: Cow::Borrowed(&inventory.slots()),
                                        carried_item: Cow::Borrowed(&player_inventory.get_cursor()),
                                    };

                                    compose.unicast(&packet, *io, system).unwrap();
                                } else {
                                    let changed_filtered = u128::from(
                                        inventory.changed & !open_inventory.client_changed,
                                    );

                                    let mut player_inventory_changed =
                                        u128::from(player_inventory.changed);

                                    player_inventory_changed >>= *(9..=44).start();

                                    player_inventory_changed <<= inventory.slots().len();

                                    let changed_filtered =
                                        changed_filtered | player_inventory_changed;

                                    if changed_filtered != 0 {
                                        for (i, slot) in inventory
                                            .slots()
                                            .iter()
                                            .chain(
                                                player_inventory
                                                    .slots()
                                                    .iter()
                                                    .skip(*(9..=44).start() as usize),
                                            )
                                            .enumerate()
                                        {
                                            if (changed_filtered >> i) & 1 == 1 {
                                                let packet = play::ScreenHandlerSlotUpdateS2c {
                                                    window_id: inv_state.window_id as i8,
                                                    state_id: VarInt::from(inv_state.state_id.0),
                                                    slot_idx: i as i16,
                                                    slot_data: Cow::Borrowed(slot),
                                                };

                                                compose.unicast(&packet, *io, system).unwrap();
                                            }
                                        }

                                        player_inventory.changed = 0;
                                    }
                                }
                                open_inventory.client_changed = 0;
                                inv_state.slots_changed = 0;
                                inventory.changed = 0;
                            })
                            .or_else(|| {
                                entity.remove::<OpenInventory>();

                                let packet = play::CloseScreenS2c {
                                    window_id: inv_state.window_id,
                                };

                                compose.unicast(&packet, *io, system).unwrap();

                                None
                            });
                    },
                );
        });

        system!(
            "update_player_selected_slot",
            world,
            &Compose($),
            &PlayerInventory,
            &ConnectionId,
        )
        .singleton()
        .with::<flecs::pipeline::PostUpdate>()
        .each_iter(|it, _, (compose, player_inventory, io)| {
            let system = it.system();

            let packet = play::UpdateSelectedSlotS2c {
                slot: player_inventory.hand_slot as u8,
            };

            compose.unicast(&packet, *io, system).unwrap();
        });

        system!(
            "update_player_inventory",
            world,
            &Compose($),
            &mut PlayerInventory,
            &mut InventoryState,
            &CursorItem,
            &ConnectionId,
        )
        .singleton()
        .without::<OpenInventory>()
        .with::<flecs::pipeline::PostUpdate>()
        .each_iter(|it, _, (compose, inventory, inv_state, cursor_item, io)| {
            let io = *io;
                let system = it.system();

                for slot in &inventory.updated_since_last_tick {
                    let slot = slot as u16;
                    let item = inventory
                        .get(slot)
                        .with_context(|| format!("failed to get item for slot {slot}")).unwrap();

                    let pkt = play::ScreenHandlerSlotUpdateS2c {
                        window_id: 0,
                        state_id: VarInt::default(),
                        slot_idx: slot as i16,
                        slot_data: Cow::Borrowed(item),
                    };
                    compose
                        .unicast(&pkt, io, system)
                        .context("failed to send inventory update").unwrap();

                    let pkt = play::ScreenHandlerSlotUpdateS2c {
                        window_id: -1,
                        state_id: VarInt::from(inv_state.state_id.0),
                        slot_idx: -1,
                        slot_data: Cow::Borrowed(&cursor_item.0),
                    };

                    compose
                        .unicast(&pkt, io, system)
                        .context("failed to send cursor item update").unwrap();
                }

                inventory.updated_since_last_tick.clear();
                inventory.hand_slot_updated_since_last_tick = false;
        });

        system!(
            "update_cursor_item",
            world,
            &Compose($),
            &mut InventoryState,
            &CursorItem,
            &ConnectionId,
        )
        .singleton()
        .with::<flecs::pipeline::PostUpdate>()
        .each_iter(|it, _, (compose, inv_state, cursor_item, io)| {
            let system = it.system();

            if inv_state.client_updated_cursor_item.as_ref() != Some(&cursor_item.0) {

                let packet = play::ScreenHandlerSlotUpdateS2c {
                    window_id: -1,
                    state_id: VarInt::from(inv_state.state_id.0),
                    slot_idx: -1,
                    slot_data: Cow::Borrowed(&cursor_item.0),
                };

                compose.unicast(&packet, *io, system).unwrap();
            }

            inv_state.client_updated_cursor_item = None;
        });
    }
}

pub fn handle_update_selected_slot(
    packet: UpdateSelectedSlotC2s,
    query: &mut PacketSwitchQuery<'_>,
) {
    if packet.slot > 8 {
        return;
    }

    query.inventory.set_cursor(packet.slot);

    let event = event::UpdateSelectedSlotEvent {
        client: query.id,
        slot: packet.slot as u8,
    };

    query.events.push(event, query.world);
}

pub fn handle_click_slot(packet: ClickSlotC2s<'_>, query: &mut PacketSwitchQuery<'_>) {
    let world = query.world;
    query
        .view
        .get::<(&mut InventoryState, Option<&mut OpenInventory>, &mut CursorItem)>(|(inv_state, open_inventory, cursor_item)| {
            let open_inv = open_inventory.as_ref().and_then(|open_inventory| {
                open_inventory
                    .entity
                    .entity_view(world)
                    .try_get::<&mut Inventory>(|inventory| 
                        // SAFETY: This is unsafe because we are extending the lifetime of the reference.
                        // Ensure that the reference remains valid for the extended lifetime.
                        unsafe { std::mem::transmute::<&mut Inventory, &'static mut Inventory>(inventory) })
            });

            if let Err(e) =
                validate_click_slot(&packet, &query.inventory, open_inv.as_deref(), &cursor_item)
            {
                debug!(
                    "Failed to validate click slot packet from {}: {}",
                    query.id, e
                );

                let client_inv = query.inventory.clone();

                let inventory_ref = open_inv.as_ref().map(|i| &**i).unwrap_or(&client_inv);
                let pkt = play::InventoryS2c {
                    window_id: if open_inv.is_some() {
                        inv_state.window_id
                    } else {
                        0
                    },
                    state_id: VarInt::from(inv_state.state_id.0),
                    slots: Cow::Borrowed(inventory_ref.slots()),
                    carried_item: Cow::Borrowed(&cursor_item.0),
                };

                query
                    .compose
                    .unicast(&pkt, query.io_ref, query.system)
                    .unwrap();
            }

            if packet.slot_idx == -999 && packet.mode == ClickMode::Click {
                // Client is dropping the cursor item by lcicking outside the window.
                let stack = std::mem::take(&mut cursor_item.0);

                if !stack.is_empty() {
                    let event = event::DropItemStackEvent {
                        client: query.id,
                        from_slot: None,
                        item: stack,
                    };
                    query.events.push(event, query.world);
                }
            } else if packet.mode == ClickMode::DropKey {
                // Droppign item by pressing key

                let entire_stack = packet.button == 1;

                if let Some(open_inv) = open_inv {
                    if inv_state.state_id.0 != packet.state_id.0 {
                        // out of sync
                        debug!("Client state id mismatch");

                        let pkt = play::InventoryS2c {
                            window_id: inv_state.window_id,
                            state_id: VarInt(inv_state.state_id.0),
                            slots: Cow::Borrowed(open_inv.slots()),
                            carried_item: Cow::Borrowed(&cursor_item.0),
                        };

                        query
                            .compose
                            .unicast(&pkt, query.io_ref, query.system)
                            .unwrap();
                    }

                    if packet.slot_idx == -999 {
                        return;
                    }

                    if (0_i16..open_inv.slots().len() as i16).contains(&packet.slot_idx) {
                        if open_inv.readonly {
                            let pkt = play::InventoryS2c {
                                window_id: inv_state.window_id,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(open_inv.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();

                            return;
                        }

                        let slot = open_inv.get(packet.slot_idx as u16).unwrap().clone();

                        if !slot.is_empty() {
                            if entire_stack || slot.count == 1 {
                                open_inv
                                    .set(packet.slot_idx as u16, ItemStack::EMPTY)
                                    .unwrap();
                            } else {
                                let count = slot.count - 1;
                                open_inv
                                    .set(
                                        packet.slot_idx as u16,
                                        ItemStack::new(slot.item, count, slot.nbt.clone()),
                                    )
                                    .unwrap();
                            };

                            let event = event::DropItemStackEvent {
                                client: query.id,
                                from_slot: Some(packet.slot_idx),
                                item: open_inv.get(packet.slot_idx as u16).unwrap().clone(),
                            };

                            query.events.push(event, query.world);
                        }
                    } else {
                        // client is dropping from their inventory

                        if query.inventory.readonly {
                            let pkt = play::InventoryS2c {
                                window_id: 0,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(query.inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();

                            return;
                        }

                        let slot_id = convert_to_player_slot_id(&open_inv, packet.slot_idx as u16);

                        let stack = query.inventory.get(slot_id).unwrap().clone();

                        if !stack.is_empty() {
                            if entire_stack || stack.count == 1 {
                                query.inventory.set(slot_id, ItemStack::EMPTY).unwrap();
                            } else {
                                let count = stack.count - 1;
                                query
                                    .inventory
                                    .set(
                                        slot_id,
                                        ItemStack::new(stack.item, count, stack.nbt.clone()),
                                    )
                                    .unwrap();
                            }

                            let event = event::DropItemStackEvent {
                                client: query.id,
                                from_slot: Some(slot_id as i16),
                                item: query.inventory.get(slot_id).unwrap().clone(),
                            };

                            query.events.push(event, query.world);
                        }
                    }
                } else {
                    // player doesnt have inventory open and dropping items

                    if query.inventory.readonly {
                        let pkt = play::InventoryS2c {
                            window_id: 0,
                            state_id: VarInt(inv_state.state_id.0),
                            slots: Cow::Borrowed(query.inventory.slots()),
                            carried_item: Cow::Borrowed(&cursor_item.0),
                        };

                        query
                            .compose
                            .unicast(&pkt, query.io_ref, query.system)
                            .unwrap();

                        return;
                    }

                    if packet.slot_idx == -999 {
                        return;
                    }

                    let stack = query.inventory.get(packet.slot_idx as u16).unwrap().clone();

                    if !stack.is_empty() {
                        if packet.button == 1 || stack.count == 1 {
                            query
                                .inventory
                                .set(packet.slot_idx as u16, ItemStack::EMPTY)
                                .unwrap();
                        } else {
                            let count = stack.count - 1;
                            query
                                .inventory
                                .set(
                                    packet.slot_idx as u16,
                                    ItemStack::new(stack.item, count, stack.nbt.clone()),
                                )
                                .unwrap();
                        }

                        let event = event::DropItemStackEvent {
                            client: query.id,
                            from_slot: Some(packet.slot_idx),
                            item: query.inventory.get(packet.slot_idx as u16).unwrap().clone(),
                        };

                        query.events.push(event, query.world);
                    }
                }
            } else {
                // player is clicking a slot

                if (packet.window_id == 0) != open_inv.is_none() {
                    warn!("window_id does not match open_inventory");
                    return;
                }
                if let Some(current_open_invetory) = open_inventory {
                    if let Some(mut target_inventory) = open_inv {
                        if inv_state.state_id.0 != packet.state_id.0 {
                            debug!("Client state id mismatch, resyncing");

                            inv_state.state_id += 1;

                            let pkt = play::InventoryS2c {
                                window_id: inv_state.window_id,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(target_inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();

                            return;
                        }

                        let mut new_cursor = packet.carried_item.clone();

                        for slot in packet.slot_changes.iter() {
                            let transferred_between_inventories = ((0_i16
                                ..target_inventory.slots().len() as i16)
                                .contains(&packet.slot_idx)
                                && packet.mode == ClickMode::Hotbar)
                                || packet.mode == ClickMode::ShiftClick;

                            if (0_i16..target_inventory.slots().len() as i16).contains(&slot.idx) {
                                if (query.inventory.readonly && transferred_between_inventories)
                                    || target_inventory.readonly
                                {
                                    new_cursor = cursor_item.0.clone();
                                    continue;
                                }

                                target_inventory.set(slot.idx as u16, slot.stack.clone()).unwrap();
                                current_open_invetory.client_changed |= 1 << slot.idx;
                            } else {
                                if (target_inventory.readonly && transferred_between_inventories)
                                    || query.inventory.readonly
                                {
                                    new_cursor = cursor_item.0.clone();
                                    continue;
                                }

                                // The client is interacting with a slot in their own inventory.
                                let slot_id = convert_to_player_slot_id(
                                    target_inventory,
                                    slot.idx as u16,
                                );
                                query.inventory.set(slot_id, slot.stack.clone()).unwrap();
                                inv_state.slots_changed |= 1 << slot_id;
                            }
                        }

                        cursor_item.0 = new_cursor.clone();
                        inv_state.client_updated_cursor_item = Some(new_cursor);

                        if target_inventory.readonly || query.inventory.readonly {
                            let pkt = play::InventoryS2c {
                                window_id: inv_state.window_id,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(target_inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();

                            let pkt = play::InventoryS2c {
                                window_id: 0,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(query.inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();
                        }
                    } else {
                        // player interacting with their own inventoryt

                        if inv_state.state_id.0 != packet.state_id.0 {
                            debug!("Client state id mismatch, resyncing");

                            inv_state.state_id += 1;

                            let pkt = play::InventoryS2c {
                                window_id: 0,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(query.inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();

                            return;
                        }

                        let mut new_cursor = packet.carried_item.clone();

                        for slot in packet.slot_changes.iter() {
                            if (0_i16..query.inventory.slots().len() as i16).contains(&slot.idx) {
                                if query.inventory.readonly {
                                    new_cursor = cursor_item.0.clone();
                                    continue;
                                }

                                query.inventory.set(slot.idx as u16, slot.stack.clone()).unwrap();
                                inv_state.slots_changed |= 1 << slot.idx;
                            }
                        }

                        cursor_item.0 = new_cursor.clone();
                        inv_state.client_updated_cursor_item = Some(new_cursor);

                        if query.inventory.readonly {
                            let pkt = play::InventoryS2c {
                                window_id: 0,
                                state_id: VarInt(inv_state.state_id.0),
                                slots: Cow::Borrowed(query.inventory.slots()),
                                carried_item: Cow::Borrowed(&cursor_item.0),
                            };

                            query
                                .compose
                                .unicast(&pkt, query.io_ref, query.system)
                                .unwrap();
                        }
                    }
                }

                // send event 
                let event = event::ClickSlotEvent {
                    client: query.id,
                    window_id: packet.window_id,
                    state_id: packet.state_id.0,
                    slot: packet.slot_idx,
                    button: packet.button,
                    mode: packet.mode,
                    slot_changes: packet.slot_changes.into(),
                    carried_item: packet.carried_item,
                };

                query.events.push(event, query.world);
            }
        });
}

fn convert_to_player_slot_id(target_kind: &Inventory, slot_id: u16) -> u16 {
    // the first slot in the player's general inventory
    let offset = target_kind.slots().len() as u16;
    slot_id - offset + 9
}

fn validate_click_slot(
    packet: &ClickSlotC2s<'_>,
    player_inventory: &PlayerInventory,
    open_inventory: Option<&Inventory>,
    cursor_item: &CursorItem,
) -> anyhow::Result<()> {
    ensure!(
        (packet.window_id == 0) == open_inventory.is_none(),
        "window_id does not match open_inventory"
    );

    let max_slot = if let Some(open_inv) = open_inventory {
        46 + open_inv.slots().len()
    } else {
        player_inventory.slots().len()
    };

    ensure!(
        packet.slot_changes.iter().all(|s| {
            if !(0..=max_slot).contains(&(s.idx as usize)) {
                return false;
            }

            if !s.stack.is_empty() {
                let max_stack_size = s.stack.item.max_stack().max(s.stack.count);
                if !(1..=max_stack_size).contains(&(s.stack.count)) {
                    return false;
                }
            }

            true
        }),
        "invalid slot ids or item counts"
    );

    // check carried item count is valid
    if !packet.carried_item.is_empty() {
        let carried_item = &packet.carried_item;

        let max_stack_size = carried_item.item.max_stack().max(carried_item.count);
        ensure!(
            (1..=max_stack_size).contains(&carried_item.count),
            "invalid carried item count"
        );
    }

    match packet.mode {
        ClickMode::Click => {
            ensure!((0..=1).contains(&packet.button), "invalid button");
            ensure!(
                (0..=max_slot).contains(&(packet.slot_idx as usize))
                    || packet.slot_idx == -999
                    || packet.slot_idx == -1,
                "invalid slot index"
            )
        }
        ClickMode::ShiftClick => {
            ensure!((0..=1).contains(&packet.button), "invalid button");
            ensure!(
                packet.carried_item.is_empty(),
                "carried item must be empty for a hotbar swap"
            );
            ensure!(
                (0..=max_slot).contains(&(packet.slot_idx as usize)),
                "invalid slot index"
            )
        }
        ClickMode::Hotbar => {
            ensure!(matches!(packet.button, 0..=8 | 40), "invalid button");
            ensure!(
                packet.carried_item.is_empty(),
                "carried item must be empty for a hotbar swap"
            );
        }
        ClickMode::CreativeMiddleClick => {
            ensure!(packet.button == 2, "invalid button");
            ensure!(
                (0..=max_slot).contains(&(packet.slot_idx as usize)),
                "invalid slot index"
            )
        }
        ClickMode::DropKey => {
            ensure!((0..=1).contains(&packet.button), "invalid button");
            ensure!(
                packet.carried_item.is_empty(),
                "carried item must be empty for an item drop"
            );
            ensure!(
                (0..=max_slot).contains(&(packet.slot_idx as usize)) || packet.slot_idx == -999,
                "invalid slot index"
            )
        }
        ClickMode::Drag => {
            ensure!(
                matches!(packet.button, 0..=2 | 4..=6 | 8..=10),
                "invalid button"
            );
            ensure!(
                (0..=max_slot).contains(&(packet.slot_idx as usize)) || packet.slot_idx == -999,
                "invalid slot index"
            )
        }
        ClickMode::DoubleClick => ensure!(packet.button == 0, "invalid button"),
    }

    // Check that items aren't being duplicated, i.e. conservation of mass.

    let window = InventoryWindow {
        player_inventory,
        open_inventory,
    };

    match packet.mode {
        ClickMode::Click => {
            if packet.slot_idx == -1 {
                // Clicked outside the allowed window
                ensure!(
                    packet.slot_changes.is_empty(),
                    "slot modifications must be empty"
                );

                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );
            } else if packet.slot_idx == -999 {
                // Clicked outside the window, so the client is dropping an item
                ensure!(
                    packet.slot_changes.is_empty(),
                    "slot modifications must be empty"
                );

                // Clicked outside the window
                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                let expected_delta = match packet.button {
                    1 => -1,
                    0 => {
                        if !cursor_item.0.is_empty() {
                            -i32::from(cursor_item.0.count)
                        } else {
                            0
                        }
                    }
                    _ => unreachable!(),
                };
                ensure!(
                    count_deltas == expected_delta,
                    "invalid item delta: expected {}, got {}",
                    expected_delta,
                    count_deltas
                );
            } else {
                // If the user clicked on an empty slot for example
                if packet.slot_changes.len() == 0 {
                    let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                    ensure!(
                        count_deltas == 0,
                        "invalid item delta: expected 0, got {}",
                        count_deltas
                    );
                } else {
                    ensure!(
                        packet.slot_changes.len() == 1,
                        "click must modify one slot, got {}",
                        packet.slot_changes.len()
                    );

                    let old_slot = window.slot(packet.slot_changes[0].idx as u16);
                    // TODO: make sure NBT is the same.
                    //       Sometimes, the client will add nbt data to an item if it's missing,
                    // like       "Damage" to a sword.
                    let should_swap: bool = packet.button == 0
                        && match (!old_slot.is_empty(), !cursor_item.is_empty()) {
                            (true, true) => old_slot.item != cursor_item.item,
                            (true, false) => true,
                            (false, true) => cursor_item.count <= cursor_item.item.max_stack(),
                            (false, false) => false,
                        };

                    if should_swap {
                        // assert that a swap occurs
                        ensure!(
                            // There are some cases where the client will add NBT data that
                            // did not previously exist.
                            old_slot.item == packet.carried_item.item
                                && old_slot.count == packet.carried_item.count
                                && cursor_item.0 == packet.slot_changes[0].stack,
                            "swapped items must match"
                        );
                    } /* else {
                        // assert that a merge occurs
                        let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                        ensure!(
                            count_deltas == 0,
                            "invalid item delta for stack merge: {}",
                            count_deltas
                        );
                    } */
                }
            }
        }
        ClickMode::ShiftClick => {
            // If the user clicked on an empty slot for example
            if packet.slot_changes.len() == 0 {
                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );
            } else {
                ensure!(
                    (2..=3).contains(&packet.slot_changes.len()),
                    "shift click must modify 2 or 3 slots, got {}",
                    packet.slot_changes.len()
                );

                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );

                let Some(item_kind) = packet
                    .slot_changes
                    .iter()
                    .find(|s| !s.stack.is_empty())
                    .map(|s| s.stack.item)
                else {
                    bail!("shift click must move an item");
                };

                let old_slot_kind = window.slot(packet.slot_idx as u16).item;
                ensure!(
                    old_slot_kind == item_kind,
                    "shift click must move the same item kind as modified slots"
                );

                // assert all moved items are the same kind
                ensure!(
                    packet
                        .slot_changes
                        .iter()
                        .filter(|s| !s.stack.is_empty())
                        .all(|s| s.stack.item == item_kind),
                    "shift click must move the same item kind"
                );
            }
        }

        ClickMode::Hotbar => {
            if packet.slot_changes.len() == 0 {
                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );
            } else {
                ensure!(
                    packet.slot_changes.len() == 2,
                    "hotbar swap must modify two slots, got {}",
                    packet.slot_changes.len()
                );

                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );

                // assert that a swap occurs
                let old_slots = [
                    window.slot(packet.slot_changes[0].idx as u16),
                    window.slot(packet.slot_changes[1].idx as u16),
                ];
                // There are some cases where the client will add NBT data that did not
                // previously exist.
                ensure!(
                    old_slots
                        .iter()
                        .any(|s| s.item == packet.slot_changes[0].stack.item
                            && s.count == packet.slot_changes[0].stack.count)
                        && old_slots
                            .iter()
                            .any(|s| s.item == packet.slot_changes[1].stack.item
                                && s.count == packet.slot_changes[1].stack.count),
                    "swapped items must match"
                );
            }
        }
        ClickMode::CreativeMiddleClick => {}
        ClickMode::DropKey => {
            if packet.slot_changes.len() == 0 {
                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );
            } else {
                ensure!(
                    packet.slot_changes.len() == 1,
                    "drop key must modify exactly one slot"
                );
                ensure!(
                    packet.slot_idx == packet.slot_changes.first().map_or(-2, |s| s.idx),
                    "slot index does not match modified slot"
                );

                let old_slot = window.slot(packet.slot_idx as u16);
                let new_slot = &packet.slot_changes[0].stack;
                let is_transmuting = match (!old_slot.is_empty(), !new_slot.is_empty()) {
                    // TODO: make sure NBT is the same.
                    // Sometimes, the client will add nbt data to an item if it's missing, like
                    // "Damage" to a sword.
                    (true, true) => old_slot.item != new_slot.item,
                    (_, false) => false,
                    (false, true) => true,
                };
                ensure!(!is_transmuting, "transmuting items is not allowed");

                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);

                let expected_delta = match packet.button {
                    0 => -1,
                    1 => {
                        if !old_slot.is_empty() {
                            -i32::from(old_slot.count)
                        } else {
                            0
                        }
                    }
                    _ => unreachable!(),
                };
                ensure!(
                    count_deltas == expected_delta,
                    "invalid item delta: expected {}, got {}",
                    expected_delta,
                    count_deltas
                );
            }
        }
        ClickMode::Drag => {
            if matches!(packet.button, 2 | 6 | 10) {
                let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
                ensure!(
                    count_deltas == 0,
                    "invalid item delta: expected 0, got {}",
                    count_deltas
                );
            } else {
                ensure!(packet.slot_changes.is_empty() && packet.carried_item == cursor_item.0);
            }
        }
        ClickMode::DoubleClick => {
            let count_deltas = calculate_net_item_delta(packet, &window, cursor_item);
            ensure!(
                count_deltas == 0,
                "invalid item delta: expected 0, got {}",
                count_deltas
            );
        }
    }

    Ok(())
}

fn get_slot<'a>(open_inventory: Option<&'a Inventory>, player_inventory: &'a PlayerInventory, idx: i16) -> &'a ItemStack {
    if let Some(open_inv) = open_inventory {
        if idx < open_inv.slots().len() as i16 {
            return &open_inv.slots()[idx as usize];
        }
    }
    &player_inventory.slots()[idx as usize]
}

/// Calculate the total difference in item counts if the changes in this packet
/// were to be applied.
///
/// Returns a positive number if items were added to the window, and a negative
/// number if items were removed from the window.
fn calculate_net_item_delta(
    packet: &ClickSlotC2s<'_>,
    window: &InventoryWindow<'_>,
    cursor_item: &CursorItem,
) -> i32 {
    let mut net_item_delta: i32 = 0;

    for slot in packet.slot_changes.iter() {
        let old_slot = window.slot(slot.idx as u16);
        let new_slot = &slot.stack;

        net_item_delta += match (!old_slot.is_empty(), !new_slot.is_empty()) {
            (true, true) => i32::from(new_slot.count) - i32::from(old_slot.count),
            (true, false) => -i32::from(old_slot.count),
            (false, true) => i32::from(new_slot.count),
            (false, false) => 0,
        };
    }

    net_item_delta += match (!cursor_item.0.is_empty(), !packet.carried_item.is_empty()) {
        (true, true) => i32::from(packet.carried_item.count) - i32::from(cursor_item.0.count),
        (true, false) => -i32::from(cursor_item.0.count),
        (false, true) => i32::from(packet.carried_item.count),
        (false, false) => 0,
    };

    net_item_delta
}