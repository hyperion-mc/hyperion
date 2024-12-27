use std::{ borrow::Cow, collections::HashMap };

use flecs_ecs::{
    core::{ Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider },
    macros::Component,
};
use hyperion::{
    simulation::{ entity_kind::EntityKind, Uuid },
    storage::GlobalEventHandlers,
    valence_protocol::{
        packets::play::{
            click_slot_c2s::ClickMode,
            close_screen_s2c::CloseScreenS2c,
            inventory_s2c::InventoryS2c,
        },
        ItemStack,
        VarInt,
    },
};
use hyperion_inventory::{ CursorItem, Inventory, InventoryState, OpenInventory };

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

        let entity = world.add_enum(EntityKind::BlockDisplay).set(uuid).set(inventory);

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
        world.get::<&mut GlobalEventHandlers>(|event_handlers| {
            let items = self.items.clone();
            event_handlers.click.register(move |query, event| {
                let system = query.system;
                let world = system.world();
                let button = event.mode;
                query.id
                    .entity_view(world)
                    .get::<(&InventoryState, &CursorItem)>(|(inv_state, cursor_item)| {
                        if event.window_id != inv_state.window_id() {
                            return;
                        }

                        let slot = usize::from(event.slot_idx);
                        let Some(item) = items.get(&slot) else {
                            return;
                        };

                        item(query.id, button);

                        let inventory = &*query.inventory;
                        let compose = query.compose;
                        let stream = query.io_ref;

                        // re-draw the inventory
                        let player_inv = inventory
                            .slots()
                            .into_iter()
                            .map(|slot| slot.stack.clone())
                            .collect();

                        let set_content_packet = InventoryS2c {
                            window_id: 0,
                            state_id: VarInt(0),
                            slots: Cow::Owned(player_inv),
                            carried_item: Cow::Borrowed(&cursor_item.0),
                        };

                        compose.unicast(&set_content_packet, stream, system).unwrap();
                    });
            });
        });
    }

    pub fn open(&self, system: EntityView<'_>, player: Entity) {
        let world = system.world();
        player.entity_view(world).set(OpenInventory::new(self.entity));
    }

    pub fn handle_close(&mut self, _player: Entity, _close_packet: CloseScreenS2c) {
        todo!()
    }
}
