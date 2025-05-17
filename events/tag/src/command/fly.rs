use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider, id};
use hyperion::{
    net::{Compose, ConnectionId, DataBundle, agnostic},
    simulation::Flight,
};
use hyperion_clap::{CommandPermission, MinecraftCommand};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "fly")]
#[command_permission(group = "Moderator")]
pub struct FlyCommand;

impl MinecraftCommand for FlyCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        world.get::<&Compose>(|compose| {
            caller
                .entity_view(world)
                .get::<(&mut Flight, &ConnectionId)>(|(flight, stream)| {
                    flight.allow = !flight.allow;
                    flight.is_flying = flight.allow && flight.is_flying;
                    caller.entity_view(world).modified(id::<Flight>());

                    let allow_flight = flight.allow;

                    let chat_packet = if allow_flight {
                        agnostic::chat("§aFlying enabled")
                    } else {
                        agnostic::chat("§cFlying disabled")
                    };

                    let mut bundle = DataBundle::new(compose, system);
                    bundle.add_packet(&chat_packet).unwrap();

                    bundle.unicast(*stream).unwrap();
                });
        });
    }
}
