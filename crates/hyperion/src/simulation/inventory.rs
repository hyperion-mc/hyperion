use std::{ borrow::Cow, cell::Cell };
use flecs_ecs::{
    core::{ flecs, EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World },
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
use tracing::debug;
use valence_protocol::{
    packets::play::{ self, ClickSlotC2s, UpdateSelectedSlotC2s },
    VarInt,
    Decode,
};
use valence_text::IntoText;

use super::{ event, handlers::PacketSwitchQuery, Player };
use crate::net::{ Compose, ConnectionId, DataBundle };

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
            &ConnectionId,
        ).each_iter(
            |it, row, (open_inventory, compose, inv_state, io)| {
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

                let open_inv = open_inventory.as_ref().and_then(|open_inventory| {
                    open_inventory.entity.entity_view(world).try_get::<&mut Inventory>(|inventory|
                        // SAFETY: This is unsafe because we are extending the lifetime of the reference.
                        // Ensure that the reference remains valid for the extended lifetime.
                        unsafe {
                            std::mem::transmute::<&mut Inventory, &'static mut Inventory>(inventory)
                        }
                    )
                });

                let (window_id, changed) = if let Some(open_inv) = open_inv.as_deref() {
                    /* let size = match open_inv.kind() {
                        WindowType::Generic9x1 => 9,
                        WindowType::Generic9x2 => 18,
                        WindowType::Generic9x3 => 27,
                        WindowType::Generic9x4 => 36,
                        WindowType::Generic9x5 => 45,
                        WindowType::Generic9x6 => 54,
                        WindowType::Generic3x3 => 9,
                        WindowType::Anvil => 3,
                        WindowType::Beacon => 1,
                        WindowType::BlastFurnace => 3,
                        WindowType::BrewingStand => 5,
                        WindowType::Crafting => 5,
                        WindowType::Enchantment => 2,
                        WindowType::Furnace => 3,
                        WindowType::Grindstone => 3,
                        WindowType::Hopper => 5,
                        WindowType::Lectern => 1,
                        WindowType::Loom => 4,
                        WindowType::Merchant => 3,
                        WindowType::ShulkerBox => 27,
                        WindowType::Smithing => 2,
                        WindowType::Smoker => 3,
                        WindowType::Cartography => 3,
                        WindowType::Stonecutter => 2,
                    }; */
                    (inv_state.window_id(), open_inv.has_changed() || inventory.has_changed())
                } else {
                    (0, inventory.has_changed())
                };

                let inventories = if let Some(open_inv) = open_inv.as_deref() {
                    let mut slots = open_inv.slots().clone();
                    slots.extend(inventory.slots().iter().cloned());
                    slots
                } else {
                    inventory.slots().clone()
                };

                let mut changed_slots: Vec<(ItemSlot, usize)> = Vec::new();
                // inventory has changed
                if changed {
                    // group the player's inventory and the open inventory together
                    {
                        let inventories_mut = if let Some(open_inv) = open_inv {
                            let slots = open_inv.slots_mut();
                            slots.append(inventory.slots_mut());
                            slots
                        } else {
                            inventory.slots_mut()
                        };
                        // loop through all slots in the inventory
                        for (idx, slot) in inventories_mut.into_iter().enumerate() {
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

    query.inventory.set_cursor(packet.slot);

    let event = event::UpdateSelectedSlotEvent {
        client: query.id,
        slot: packet.slot as u8,
    };

    query.events.push(event, query.world);
}

pub fn handle_click_slot(packet: ClickSlotC2s<'_>, query: &mut PacketSwitchQuery<'_>) {}

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
