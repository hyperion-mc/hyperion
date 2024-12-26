use std::borrow::Cow;

use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::{
    ItemKind, ItemStack,
    net::{Compose, ConnectionId},
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_inventory::{InventoryState, PlayerInventory};
use valence_protocol::packets::play;
use valence_server::entity::abstract_fireball::Item;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "bow")]
#[command_permission(group = "Normal")]
pub struct BowCommand;

impl MinecraftCommand for BowCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();

        caller
            .entity_view(world)
            .get::<&mut PlayerInventory>(|inventory| {
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
                /* inventory.set_slot(36, ItemStack {
                    item: ItemKind::Bow,
                    count: 1,
                    nbt: None,
                });

                inventory.set_slot(37, ItemStack {
                    item: ItemKind::Arrow,
                    count: 64,
                    nbt: None,
                }); */


            });
    }
}
