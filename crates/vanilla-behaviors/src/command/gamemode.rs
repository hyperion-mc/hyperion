use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::{
    net::{agnostic, Compose, ConnectionId, DataBundle},
    simulation::Gamemode,
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use valence_server::GameMode;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "gamemode")]
#[command_permission(group = "Moderator")]
pub struct GamemodeCommand {
    #[arg(value_enum)]
    mode: hyperion_clap::GameMode,
}

impl MinecraftCommand for GamemodeCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        world.get::<&Compose>(|compose| {
            caller
                .entity_view(world)
                .get::<(&mut Gamemode, &ConnectionId)>(|(gamemode, stream)| {
                    let new_mode = match self.mode {
                        hyperion_clap::GameMode::Survival => GameMode::Survival,
                        hyperion_clap::GameMode::Adventure => GameMode::Adventure,
                        hyperion_clap::GameMode::Spectator => GameMode::Spectator,
                        hyperion_clap::GameMode::Creative => GameMode::Creative,
                    };

                    let chat_packet = if new_mode == gamemode.current {
                        agnostic::chat("§4Nothing changed")
                    } else {
                        agnostic::chat(format!("Changed gamemode to {:?}", new_mode))
                    };

                    gamemode.current = new_mode;
                    caller.entity_view(world).modified::<Gamemode>();

                    let mut bundle = DataBundle::new(compose, system);
                    bundle.add_packet(&chat_packet).unwrap();

                    bundle.unicast(*stream).unwrap();
                });
        });
    }
}
