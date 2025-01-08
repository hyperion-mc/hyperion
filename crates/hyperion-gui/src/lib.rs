#![feature(thread_local)]

use std::{borrow::Cow, cell::Cell, collections::HashMap};

use anyhow::Context;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::{
    net::{Compose, ConnectionId},
    simulation::{handlers::PacketSwitchQuery, packet::HandlerRegistry},
    valence_protocol::{
        ItemStack, VarInt,
        packets::play::{
            ClickSlotC2s,
            click_slot_c2s::ClickMode,
            close_screen_s2c::CloseScreenS2c,
            inventory_s2c::InventoryS2c,
            open_screen_s2c::{OpenScreenS2c, WindowType},
        },
        text::IntoText,
    },
};
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

#[derive(Clone)]
pub struct Gui {
    items: HashMap<usize, GuiItem>,
    size: usize,
    title: String,
    window_id: u8,
    container_type: ContainerType,
}

#[derive(Clone)]
pub struct GuiItem {
    item: ItemStack,
    on_click: fn(Entity, ClickMode),
}

/// Thread-local non-zero id means that it will be very unlikely that one player will have two
/// of the same IDs at the same time when opening GUIs in succession.
///
/// We are skipping 0 because it is reserved for the player's inventory.
fn non_zero_window_id() -> u8 {
    #[thread_local]
    static ID: Cell<u8> = Cell::new(0);

    ID.set(ID.get().wrapping_add(1));

    if ID.get() == 0 {
        ID.set(1);
    }

    ID.get()
}

impl Gui {
    #[must_use]
    pub fn new(size: usize, title: String, container_type: ContainerType) -> Self {
        Self {
            window_id: non_zero_window_id(),
            title,
            size,
            container_type,
            items: HashMap::new(),
        }
    }

    #[must_use]
    pub const fn get_window_type(&self) -> WindowType {
        match self.container_type {
            ContainerType::Chest => WindowType::Generic9x3,
            ContainerType::ShulkerBox => WindowType::ShulkerBox,
            ContainerType::Furnace => WindowType::Furnace,
            ContainerType::Dispenser => WindowType::Generic3x3,
            ContainerType::Hopper => WindowType::Hopper,
        }
    }

    pub fn add_item(&mut self, slot: usize, item: GuiItem) -> Result<(), String> {
        if slot >= self.size {
            return Err(format!(
                "Slot {} is out of bounds for GUI of size {}",
                slot, self.size
            ));
        }

        self.items.insert(slot, item);

        Ok(())
    }

    pub fn draw<'a>(&'a self, system: EntityView<'_>, player: Entity) {
        let container_items: Cow<'a, [ItemStack]> = (0..self.size)
            .map(|slot| {
                self.items
                    .get(&slot)
                    .map(|gui_item| gui_item.item.clone())
                    .unwrap_or_default()
            })
            .collect();

        let binding = ItemStack::default();
        let set_content_packet = InventoryS2c {
            window_id: self.window_id,
            state_id: VarInt(0),
            slots: container_items,
            carried_item: Cow::Borrowed(&binding),
        };

        let world = system.world();

        world.get::<&Compose>(|compose| {
            player.entity_view(world).get::<&ConnectionId>(|stream| {
                compose
                    .unicast(&set_content_packet, *stream, system)
                    .unwrap();
            });
        });
    }

    pub fn open(&mut self, system: EntityView<'_>, player: Entity) {
        let open_screen_packet = OpenScreenS2c {
            window_id: VarInt(i32::from(self.window_id)),
            window_type: self.get_window_type(),
            window_title: self.title.clone().into_cow_text(),
        };

        let world = system.world();

        world.get::<&Compose>(|compose| {
            player.entity_view(world).get::<&ConnectionId>(|stream| {
                compose
                    .unicast(&open_screen_packet, *stream, system)
                    .unwrap();
            });
        });

        self.draw(system, player);

        world.get::<&mut HandlerRegistry>(|registry| {
            let window_id = self.window_id;
            let items = self.items.clone();
            let gui = self.clone();
            registry.add_handler(Box::new(
                move |event: &ClickSlotC2s<'_>,
                      _: &dyn LifetimeHandle<'_>,
                      query: &mut PacketSwitchQuery<'_>| {
                    let system = query.system;
                    let button = event.mode;

                    if event.window_id != window_id {
                        return Ok(());
                    }

                    let slot = usize::try_from(event.slot_idx).context("invalid slot index")?;
                    let Some(item) = items.get(&slot) else {
                        return Ok(());
                    };

                    (item.on_click)(player, button);
                    gui.draw(query.system, player);

                    let inventory = &*query.inventory;
                    let compose = query.compose;
                    let stream = query.io_ref;

                    // re-draw the inventory
                    let player_inv = inventory.slots();

                    let set_content_packet = InventoryS2c {
                        window_id: 0,
                        state_id: VarInt(0),
                        slots: Cow::Borrowed(player_inv),
                        carried_item: Cow::Borrowed(&ItemStack::EMPTY),
                    };

                    compose
                        .unicast(&set_content_packet, stream, system)
                        .unwrap();

                    Ok(())
                },
            ));
        });
    }

    pub fn handle_close(&mut self, _player: Entity, _close_packet: CloseScreenS2c) {
        todo!()
    }
}

impl GuiItem {
    pub fn new(item: ItemStack, on_click: fn(Entity, ClickMode)) -> Self {
        Self { item, on_click }
    }
}
