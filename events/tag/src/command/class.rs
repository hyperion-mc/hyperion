use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::net::{Compose, ConnectionId, agnostic};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_rank_tree::{Class, Team};
use tracing::error;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "class")]
#[command_permission(group = "Normal")]
pub struct ClassCommand {
    class: Class,
    team: Team,
}
impl MinecraftCommand for ClassCommand {
    type State = SystemState<(
        Res<'static, Compose>,
        Query<'static, 'static, (&'static ConnectionId, &'static Team, &'static Class)>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let (compose, query, mut commands) = state.get(world);
        let class_param = self.class;
        let team_param = self.team;

        let (&connection_id, team, class) = match query.get(caller) {
            Ok(data) => data,
            Err(e) => {
                error!("class command failed: query failed: {e}");
                return;
            }
        };

        if *team == team_param && *class == class_param {
            let chat_pkt = agnostic::chat("§cYou’re already using this class!");

            compose.unicast(&chat_pkt, connection_id).unwrap();
            return;
        }

        commands
            .entity(caller)
            .insert(team_param)
            .insert(class_param);

        let msg = format!("Setting rank to {class_param:?}");
        let chat = agnostic::chat(msg);
        compose.unicast(&chat, connection_id).unwrap();
    }
}
