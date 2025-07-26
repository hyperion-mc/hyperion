use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{
    net::{Compose, ConnectionId, agnostic},
    simulation::FlyingSpeed,
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use tracing::error;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "speed")]
#[command_permission(group = "Moderator")]
pub struct SpeedCommand {
    amount: f32,
}

impl MinecraftCommand for SpeedCommand {
    type State = SystemState<(
        Query<'static, 'static, &'static ConnectionId>,
        Res<'static, Compose>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (query, compose, mut commands) = state.get(world);

        let &connection_id = match query.get(caller) {
            Ok(connection_id) => connection_id,
            Err(e) => {
                error!("speed command failed: query failed: {e}");
                return;
            }
        };

        let msg = format!("Setting speed to {}", self.amount);
        let chat = agnostic::chat(msg);
        compose.unicast(&chat, connection_id).unwrap();

        commands
            .entity(caller)
            .insert(FlyingSpeed::new(self.amount));
    }
}
