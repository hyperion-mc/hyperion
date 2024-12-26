use flecs_ecs::{
    core::World,
    macros::Component,
    prelude::Module,
};
use hyperion_inventory::OpenInventory;
use valence_protocol::packets::play::{ClickSlotC2s, UpdateSelectedSlotC2s};

use super::{event, handlers::PacketSwitchQuery};

#[derive(Component)]
pub struct InventoryModule;

impl Module for InventoryModule {
    fn module(world: &World) {
        world.component::<OpenInventory>();
    }
}

pub fn handle_update_selected_slot(
    packet: UpdateSelectedSlotC2s,
    query: &mut PacketSwitchQuery<'_>,
) {
    /* if packet.slot > 8 {
        return;
    }

    query.inventory.set_cursor(packet.slot);

    let event = event::UpdateSelectedSlotEvent {
        client: query.id,
        slot: packet.slot as u8,
    };

    query.events.push(event, query.world); */
}

pub fn handle_click_slot(packet: ClickSlotC2s<'_>, query: &mut PacketSwitchQuery<'_>) {}
