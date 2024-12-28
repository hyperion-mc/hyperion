use clap::Parser;
use flecs_ecs::core::{ Builder, Entity, EntityView, QueryAPI, WorldProvider };
use hyperion::{ simulation::entity_kind::EntityKind, ItemKind, ItemStack };
use hyperion_clap::{ CommandPermission, MinecraftCommand };
use hyperion_gui::Gui;
use hyperion_inventory::Inventory;
use tracing::debug;
use valence_protocol::packets::play::{ click_slot_c2s::ClickMode, open_screen_s2c::WindowType };
use valence_server::entity::abstract_fireball::Item;

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
                return;
            }
        });

        if !found {
            debug!("Creating new GUI");
            let mut gui_inventory = Inventory::new(
                27,
                "Test Chest".to_string(),
                WindowType::Generic9x3,
                false
            );

            let item = ItemStack::new(ItemKind::GoldIngot, 64, None);

            gui_inventory.set(13, item).unwrap();
            gui_inventory.set(14, ItemStack::new(ItemKind::Diamond, 64, None)).unwrap();
            gui_inventory.set(15, ItemStack::new(ItemKind::IronIngot, 64, None)).unwrap();
            gui_inventory.set(16, ItemStack::new(ItemKind::Coal, 64, None)).unwrap();
            gui_inventory.set(17, ItemStack::new(ItemKind::Emerald, 64, None)).unwrap();
            gui_inventory.set(18, ItemStack::new(ItemKind::GoldIngot, 64, None)).unwrap();

            let gui = Gui::new(gui_inventory, &world, 28);
            /* gui.add_command(13, |player, click_mode| match click_mode {
                ClickMode::Click => debug!("Left Click"),
                ClickMode::ShiftClick => debug!("Shift Click"),
                ClickMode::Hotbar => debug!("Hotbar"),
                ClickMode::CreativeMiddleClick => debug!("Creative Middle Click"),
                ClickMode::DropKey => debug!("Drop Key"),
                ClickMode::Drag => debug!("Drag"),
                ClickMode::DoubleClick => debug!("Double Click"),
            }); */

            gui.open(system, caller);
            // add the gui to the world
            world.add_enum(EntityKind::Gui).set(gui);
        }
    }
}
