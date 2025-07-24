use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{ItemKind, ItemStack, simulation::entity_kind::EntityKind};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_gui::Gui;
use hyperion_inventory::Inventory;
use tracing::debug;
use valence_protocol::packets::play::{click_slot_c2s::ClickMode, open_screen_s2c::WindowType};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "testgui")]
#[command_permission(group = "Normal")]
pub struct GuiCommand;

impl MinecraftCommand for GuiCommand {
    type State = SystemState<(
        Query<'static, 'static, &'static Gui>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (query, mut commands) = state.get(world);

        for gui in &query {
            if gui.id == 27 {
                gui.open_deferred(&mut commands, caller);
                return;
            }
        }

        // The gui was not found, so create one
        let mut gui_inventory =
            Inventory::new(27, "Test GUI".to_string(), WindowType::Generic9x3, true);

        let item = ItemStack::new(ItemKind::GoldIngot, 1, None);

        gui_inventory.set(13, item).unwrap();

        commands.queue(move |world: &mut World| {
            let mut gui = Gui::new(gui_inventory, world, 27);
            gui.add_command(13, |player, click_mode| match click_mode {
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
            });

            gui.open(world, caller);

            world.spawn((EntityKind::Gui, gui));
        });
    }
}
