use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{
    net::{Compose, ConnectionId, DataBundle, agnostic},
    simulation::Flight,
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use tracing::error;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "fly")]
#[command_permission(group = "Moderator")]
pub struct FlyCommand;

impl MinecraftCommand for FlyCommand {
    type State = SystemState<(
        Res<'static, Compose>,
        Query<'static, 'static, (&'static ConnectionId, &'static Flight)>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (compose, query, mut commands) = state.get(world);

        let (&connection_id, &(mut flight)) = match query.get(caller) {
            Ok(data) => data,
            Err(e) => {
                error!("fly command failed: query failed: {e}");
                return;
            }
        };

        flight.allow = !flight.allow;
        flight.is_flying = flight.allow && flight.is_flying;

        let allow_flight = flight.allow;

        let chat_packet = if allow_flight {
            agnostic::chat("§aFlying enabled")
        } else {
            agnostic::chat("§cFlying disabled")
        };

        let mut bundle = DataBundle::new(&compose);
        bundle.add_packet(&chat_packet).unwrap();
        bundle.unicast(connection_id).unwrap();

        commands.entity(caller).insert(flight);
    }
}
