use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldProvider};
use hyperion::{ItemKind, ItemStack};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_inventory::PlayerInventory;

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
            });
    }
}
