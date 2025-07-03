use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::net::{Compose, ConnectionId};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use tracing::error;

use crate::plugin::vanish::Vanished;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "vanish")]
#[command_permission(group = "Admin")]
pub struct VanishCommand;

impl MinecraftCommand for VanishCommand {
    type State = SystemState<(
        Query<
            'static,
            'static,
            (
                &'static ConnectionId,
                &'static Name,
                Option<&'static Vanished>,
            ),
        >,
        Res<'static, Compose>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (query, compose, mut commands) = state.get(world);

        let (&connection_id, name, vanished) = match query.get(caller) {
            Ok(data) => data,
            Err(e) => {
                error!("vanish command failed: query failed: {e}");
                return;
            }
        };

        let is_vanished = !vanished.is_some_and(Vanished::is_vanished);

        commands.entity(caller).insert(Vanished::new(is_vanished));

        let packet = hyperion::net::agnostic::chat(format!(
            "§7[Admin] §f{name} §7is now {}",
            if is_vanished { "vanished" } else { "visible" }
        ));
        compose.unicast(&packet, connection_id).unwrap();
    }
}
