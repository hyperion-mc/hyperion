use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::{
    net::{Compose, ConnectionId, agnostic},
    simulation::FlyingSpeed,
};
use hyperion_clap::{CommandPermission, MinecraftCommand};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "speed")]
#[command_permission(group = "Moderator")]
pub struct SpeedCommand {
    amount: f32,
}

impl MinecraftCommand for SpeedCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let msg = format!("Setting speed to {}", self.amount);
        let chat = agnostic::chat(msg);

        world.get::<&Compose>(|compose| {
            caller.entity_view(world).get::<&ConnectionId>(|stream| {
                caller.entity_view(world).set(FlyingSpeed::new(self.amount));
                compose.unicast(&chat, *stream, system).unwrap();
            });
        });
    }
}
