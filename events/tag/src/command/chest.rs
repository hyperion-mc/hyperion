use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{ItemKind, ItemStack, simulation::entity_kind::EntityKind};
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
    type State = SystemState<(
        Query<'static, 'static, &'static Gui>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (query, mut commands) = state.get(world);

        for gui in &query {
            if gui.id == 28 {
                gui.open_deferred(&mut commands, caller);
                return;
            }
        }

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

        commands.queue(move |world: &mut World| {
            let gui = Gui::new(gui_inventory, world, 28);

            gui.open(world, caller);

            world.spawn((EntityKind::Gui, gui));
        });
    }
}
