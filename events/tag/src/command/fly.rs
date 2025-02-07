use flecs_ecs::core::ComponentOrPairId;
use clap::Parser;
use flecs_ecs::{
    core::{Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider, flecs},
    macros::Component,
};
use hyperion::{
    net::{Compose, ConnectionId, DataBundle, agnostic},
    simulation::Player,
    valence_protocol::packets::play::{
        PlayerAbilitiesS2c, player_abilities_s2c::PlayerAbilitiesFlags,
    },
};
use hyperion_clap::{CommandPermission, MinecraftCommand};

#[derive(Component)]
#[meta]
pub struct Flight {
    pub allow: bool,
}

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

                    let allow_flight = flight.allow;

                    let chat_packet = if allow_flight {
                        agnostic::chat("§aFlying enabled")
                    } else {
                        agnostic::chat("§cFlying disabled")
                    };

                    let packet = fly_packet(allow_flight);

                    let mut bundle = DataBundle::new(compose, system);
                    bundle.add_packet(&packet).unwrap();
                    bundle.add_packet(&chat_packet).unwrap();

                    bundle.unicast(*stream).unwrap();
                });
        });
    }

    fn pre_register(world: &World) {
        // register the component with meta meaning we can view the value in the flecs
        // explorer UI
        world.component::<Flight>().meta();

        // whenever a Player component is added, we add the Flight component to them.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Flight)>();
    }
}

fn fly_packet(allow_flight: bool) -> PlayerAbilitiesS2c {
    const SPEED_METER_PER_SECOND: f32 = 10.92;

    // guessing.. idk what the actual conversion is
    const SOME_CONVERSION: f32 = SPEED_METER_PER_SECOND / 70.0;

    PlayerAbilitiesS2c {
        flags: PlayerAbilitiesFlags::default()
            .with_flying(allow_flight)
            .with_allow_flying(allow_flight),
        flying_speed: SOME_CONVERSION,
        fov_modifier: 0.0,
    }
}
