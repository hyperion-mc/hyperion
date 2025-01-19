use std::collections::HashMap;

use flecs_ecs::{
    core::{Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider},
    macros::Component,
};
use hyperion::{
    simulation::{
        Spawn, Uuid, entity_kind::EntityKind, handlers::PacketSwitchQuery, packet::HandlerRegistry,
    },
    valence_protocol::packets::play::{
        ClickSlotC2s, click_slot_c2s::ClickMode, close_screen_s2c::CloseScreenS2c,
    },
};
use hyperion_inventory::{Inventory, InventoryState, OpenInventory};
use hyperion_utils::LifetimeHandle;
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
    pub fn new(inventory: Inventory, world: &World, id: u64) -> Self {
        let uuid = Uuid::new_v4();

        let entity = world
            .entity()
            .add_enum(EntityKind::BlockDisplay)
            .set(uuid)
            .set(inventory);

        entity.enqueue(Spawn);

        Self {
            entity: *entity,
            items: HashMap::new(),
            id,
        }
    }

    pub fn add_command(&mut self, slot: usize, on_click: fn(Entity, ClickMode)) {
        self.items.insert(slot, on_click);
    }

    pub fn init(&mut self, world: &World) {
        world.get::<&mut HandlerRegistry>(|registry| {
            let items = self.items.clone();
            registry.add_handler(Box::new(
                move |event: &ClickSlotC2s<'_>,
                      _: &dyn LifetimeHandle<'_>,
                      query: &mut PacketSwitchQuery<'_>| {
                    let system = query.system;
                    let world = system.world();
                    let button = event.mode;
                    query
                        .id
                        .entity_view(world)
                        .get::<&InventoryState>(|inv_state| {
                            if event.window_id != inv_state.window_id() {
                                return;
                            }

                            let Ok(slot) = usize::try_from(event.slot_idx) else {
                                return;
                            };
                            let Some(item) = items.get(&slot) else {
                                return;
                            };

                            item(query.id, button);
                        });

                    Ok(())
                },
            ));
        });
    }

    pub fn open(&self, system: EntityView<'_>, player: Entity) {
        let world = system.world();
        player
            .entity_view(world)
            .set(OpenInventory::new(self.entity));
    }

    pub fn handle_close(&mut self, _player: Entity, _close_packet: CloseScreenS2c) {
        todo!()
    }
}
