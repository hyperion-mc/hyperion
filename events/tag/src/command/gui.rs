use clap::Parser;
use flecs_ecs::core::{ Builder, Entity, EntityView, QueryAPI, WorldProvider };
use hyperion::{ ItemKind, ItemStack };
use hyperion_clap::{ CommandPermission, MinecraftCommand };
use hyperion_gui::Gui;
use hyperion_inventory::Inventory;
use tracing::debug;
use valence_protocol::packets::play::{ click_slot_c2s::ClickMode, open_screen_s2c::WindowType };

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "testgui")]
#[command_permission(group = "Normal")]
pub struct GuiCommand;

impl MinecraftCommand for GuiCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        /* let mut gui = Gui::new(27, "Test Chest GUI".to_string(), ContainerType::Chest);

        let info_item = GuiItem::new(
            ItemBuilder::new(hyperion::ItemKind::GoldIngot)
                .name("Information")
                .glowing()
                .build(),
            |_player, click_mode| match click_mode {
                ClickMode::Click => debug!("Left Click"),
                ClickMode::ShiftClick => debug!("Shift Click"),
                ClickMode::Hotbar => debug!("Hotbar"),
                ClickMode::CreativeMiddleClick => debug!("Creative Middle Click"),
                ClickMode::DropKey => debug!("Drop Key"),
                ClickMode::Drag => debug!("Drag"),
                ClickMode::DoubleClick => debug!("Double Click"),
            },
        );

        gui.add_item(13, info_item).unwrap(); */
        let world = system.world();
        /* world.get::<&Gui>(|gui| {}); */
        // get a list of all the guis
        let gui = world.query::<&Gui>().build();
        let mut found = false;
        gui.each_iter(|_it, _, gui| {
            if gui.id == 27 {
                gui.open(system, caller);
                found = true;
                return;
            }
        });
        if !found {
            let mut gui_inventory = Inventory::new(
                27,
                "Test GUI".to_string(),
                WindowType::Generic9x3,
                true
            );

            let item = ItemStack::new(ItemKind::GoldIngot, 1, None);

            gui_inventory.set(13, item).unwrap();

            let mut gui = Gui::new(gui_inventory, &world, 27);
            gui.add_command(13, |player, click_mode| {
                match click_mode {
                    ClickMode::Click => {
                        debug!("Player {:?} clicked on slot 13", player);
                    }
                    ClickMode::DoubleClick => {
                        debug!("Player {:?} double clicked on slot 13", player);
                    }
                    ClickMode::Drag => {
                        debug!("Player {:?} dragged on slot 13", player);
                    }
                    ClickMode::DropKey => {
                        debug!("Player {:?} dropped on slot 13", player);
                    }
                    ClickMode::Hotbar => {
                        debug!("Player {:?} hotbar on slot 13", player);
                    }
                    ClickMode::ShiftClick => {
                        debug!("Player {:?} shift clicked on slot 13", player);
                    }
                    ClickMode::CreativeMiddleClick => {
                        debug!("Player {:?} creative middle clicked on slot 13", player);
                    }
                }
            });

            gui.init(&world);

            gui.open(system, caller);
        }

        // gui.open(system, caller);
    }
}
