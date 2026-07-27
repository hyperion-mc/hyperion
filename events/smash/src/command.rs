//! Chat commands, which are the only way to pick a kit until there is a menu.
//!
//! Mineplex used a compass and an inventory GUI in the hub. `/kit <name>` is the
//! same choice made through the one input surface that already works.

use std::fmt::Write;

use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider};
use hyperion::net::{Compose, ConnectionId, agnostic};
use hyperion_clap::{CommandPermission, MinecraftCommand, hyperion_command::CommandRegistry};

use crate::module::{
    kit::{self, KitBlurb, KitName},
    lobby,
};

pub fn register(registry: &mut CommandRegistry, world: &World) {
    KitCommand::register(registry, world);
    KitsCommand::register(registry, world);
}

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "kit")]
#[command_permission(group = "Normal")]
pub struct KitCommand {
    /// The kit to play, as shown by `/kits`.
    name: String,
}

impl MinecraftCommand for KitCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let player = caller.entity_view(world);

        let message = match lobby::select_kit(&world, player, &self.name) {
            // select_kit already sends the confirmation and the new hotbar
            // through the seam, so there is nothing left to say here.
            Ok(()) => return,
            Err(reason) => format!("§c{reason}"),
        };

        tell(world, caller, &message);
    }
}

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "kits")]
#[command_permission(group = "Normal")]
pub struct KitsCommand;

impl MinecraftCommand for KitsCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();

        let mut listing = String::from("§6Kits:");
        for id in kit::registry(&world) {
            let entry = world.entity_from_id(id);
            let Some(name) = entry.try_get::<&KitName>(|name| name.0) else {
                continue;
            };
            let blurb = entry.try_get::<&KitBlurb>(|blurb| blurb.0).unwrap_or("");
            let _unused = write!(listing, "\n§e{name}§7 — {blurb}");
        }

        tell(world, caller, &listing);
    }
}

fn tell(world: flecs_ecs::core::WorldRef<'_>, caller: Entity, message: &str) {
    let chat = agnostic::chat(message);
    world.get::<&Compose>(|compose| {
        caller.entity_view(world).get::<&ConnectionId>(|stream| {
            if let Err(error) = compose.unicast(&chat, *stream) {
                tracing::warn!("dropping a smash command reply: {error}");
            }
        });
    });
}
