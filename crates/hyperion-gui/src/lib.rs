use std::collections::HashMap;

use bevy::prelude::*;
use hyperion::{
    simulation::{Uuid, entity_kind::EntityKind},
    valence_protocol::packets::play::{
        click_slot_c2s::ClickMode, close_screen_s2c::CloseScreenS2c,
    },
};
use hyperion_inventory::{Inventory, OpenInventory};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryItem {
    pub id: String,
    pub name: String,
    pub lore: Option<String>,
    pub quantity: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum ContainerType {
    Chest,
    ShulkerBox,
    Furnace,
    Dispenser,
    Hopper,
}

#[derive(Component, Clone)]
pub struct Gui {
    entity: Entity,
    items: HashMap<usize, fn(Entity, ClickMode)>,
    pub id: u64,
}

impl Gui {
    #[must_use]
    pub fn new(inventory: Inventory, world: &mut World, id: u64) -> Self {
        let uuid = Uuid::new_v4();

        let entity = world
            .spawn((EntityKind::BlockDisplay, uuid, inventory))
            .id();

        Self {
            entity,
            items: HashMap::new(),
            id,
        }
    }

    pub fn add_command(&mut self, slot: usize, on_click: fn(Entity, ClickMode)) {
        self.items.insert(slot, on_click);
    }

    pub fn init(&mut self, _world: &mut World) {
        todo!()
    }

    pub fn open(&self, world: &mut World, player: Entity) {
        world
            .entity_mut(player)
            .insert(OpenInventory::new(self.entity));
    }

    pub fn open_deferred(&self, commands: &mut Commands<'_, '_>, player: Entity) {
        commands
            .entity(player)
            .insert(OpenInventory::new(self.entity));
    }

    pub fn handle_close(&mut self, _player: Entity, _close_packet: CloseScreenS2c) {
        todo!()
    }
}
