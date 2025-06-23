use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{ItemKind, ItemStack};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_inventory::PlayerInventory;
use tracing::error;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "bow")]
#[command_permission(group = "Normal")]
pub struct BowCommand;

impl MinecraftCommand for BowCommand {
    type State = SystemState<Commands<'static, 'static>>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let mut commands = state.get(world);
        commands
            .entity(caller)
            .queue(|mut caller: EntityWorldMut<'_>| {
                let Some(mut inventory) = caller.get_mut::<PlayerInventory>() else {
                    error!("bow command failed: player is missing PlayerInventory component");
                    return;
                };

                inventory.try_add_item(ItemStack {
                    item: ItemKind::Bow,
                    count: 1,
                    nbt: None,
                });

                inventory.try_add_item(ItemStack {
                    item: ItemKind::Arrow,
                    count: 64,
                    nbt: None,
                });
            });
    }
}
