use clap::Parser;
use flecs_ecs::{
    core::{Entity, EntityView, EntityViewGet, WorldProvider},
    prelude::*,
};
use hyperion::net::{Compose, ConnectionId};
use hyperion_clap::{CommandPermission, MinecraftCommand};

use crate::module::vanish::Vanished;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "vanish")]
#[command_permission(group = "Admin")]
pub struct VanishCommand;

impl MinecraftCommand for VanishCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();

        world.get::<&Compose>(|compose| {
            caller.entity_view(world).get::<(
                Option<&Vanished>,
                &ConnectionId,
                &hyperion::simulation::Name,
            )>(|(vanished, stream, name)| {
                let is_vanished = vanished.is_some_and(Vanished::is_vanished);
                let caller = caller.entity_view(world);
                if is_vanished {
                    caller.set(Vanished::new(false));
                    let packet = hyperion::net::agnostic::chat(format!(
                        "§7[Admin] §f{name} §7is now visible",
                    ));
                    compose.unicast(&packet, *stream).unwrap();
                } else {
                    caller.set(Vanished::new(true));
                    let packet = hyperion::net::agnostic::chat(format!(
                        "§7[Admin] §f{name} §7is now vanished",
                    ));
                    compose.unicast(&packet, *stream).unwrap();
                }
            });
        });
    }
}
