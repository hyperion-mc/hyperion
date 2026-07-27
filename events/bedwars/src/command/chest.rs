use clap::Parser;
use flecs_ecs::core::{Builder, Entity, EntityView, QueryAPI, WorldProvider};
use hyperion::{
    ItemKind, ItemStack,
    simulation::{Spawn, entity_kind::EntityKind},
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_gui::Gui;
use hyperion_inventory::{Inventory, ItemSlot};
use tracing::debug;
use valence_protocol::packets::play::open_screen_s2c::WindowType;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "chest")]
#[command_permission(group = "Normal")]
pub struct ChestCommand;

impl MinecraftCommand for ChestCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();

        let gui = world.query::<&Gui>().build();
        let mut found = false;
        gui.each_iter(|_it, _, gui| {
            if gui.id == 28 {
                gui.open(system, caller);
                found = true;
            }
        });

        if !found {
            debug!("Creating new GUI");
            let mut gui_inventory =
                Inventory::new(27, "Test Chest".to_string(), WindowType::Generic9x3, false);

            let item = ItemStack::new(ItemKind::GoldIngot, 64, None);

            gui_inventory.set(13, item).unwrap();
            gui_inventory
                .set(14, ItemStack::new(ItemKind::Diamond, 64, None))
                .unwrap();
            gui_inventory
                .set(15, ItemStack::new(ItemKind::IronIngot, 64, None))
                .unwrap();
            gui_inventory
                .set(16, ItemStack::new(ItemKind::Coal, 64, None))
                .unwrap();
            gui_inventory
                .set(17, ItemStack::new(ItemKind::Emerald, 64, None))
                .unwrap();
            gui_inventory
                .set(18, ItemStack::new(ItemKind::GoldIngot, 64, None))
                .unwrap();
            gui_inventory
                .set_slot(19, ItemSlot::new(ItemKind::Diamond, 64, None, Some(true)))
                .unwrap();

            let gui = Gui::new(gui_inventory, &world, 28);

            gui.open(system, caller);
            // add the gui to the world
            world
                .entity()
                .add_enum(EntityKind::Gui)
                .set(gui)
                .enqueue(Spawn);
        }
    }
}
