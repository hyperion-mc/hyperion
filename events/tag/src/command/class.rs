use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::net::{Compose, ConnectionId, agnostic};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use hyperion_rank_tree::{Class, Team};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "class")]
#[command_permission(group = "Normal")]
pub struct ClassCommand {
    class: Class,
    team: Team,
}
impl MinecraftCommand for ClassCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let class_param = self.class;
        let team_param = self.team;

        let world = system.world();

        world.get::<&Compose>(|compose| {
            let caller = caller.entity_view(world);
            caller.get::<(&ConnectionId, &mut Team, &mut Class)>(|(stream, team, class)| {
                if *team == team_param && *class == class_param {
                    let chat_pkt = agnostic::chat("§cYou’re already using this class!");

                    compose.unicast(&chat_pkt, *stream, system).unwrap();
                    return;
                }

                if *team != team_param {
                    *team = team_param;
                    caller.modified::<Team>();
                }

                if *class != class_param {
                    *class = class_param;
                    caller.modified::<Class>();
                }

                let msg = format!("Setting rank to {class:?}");
                let chat = agnostic::chat(msg);
                compose.unicast(&chat, *stream, system).unwrap();
            });
        });
    }
}
