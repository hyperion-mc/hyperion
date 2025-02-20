use hyperion::{
    flecs_ecs::{
        self,
        core::{EntityViewGet, World, WorldGet},
        macros::Component,
        prelude::Module,
    },
    net::{ConnectionId, DataBundle},
    protocol::{
        game_mode::OptGameMode,
        packets::play::{self, PlayerAbilitiesS2c},
        BlockPos, ByteAngle, GlobalPos, VarInt,
    },
    server::{abilities::PlayerAbilitiesFlags, ident, GameMode},
    simulation::{
        event::{ClientStatusCommand, ClientStatusEvent},
        handlers::PacketSwitchQuery,
        metadata::{entity::Pose, living_entity::Health},
        packet::HandlerRegistry,
        Flight, FlyingSpeed, Pitch, Position, Uuid, Xp, Yaw,
    },
};
use hyperion_utils::{EntityExt, LifetimeHandle};

#[derive(Component)]
pub struct RespawnModule;

impl Module for RespawnModule {
    fn module(world: &World) {
        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |event: &ClientStatusEvent,
                 _: &dyn LifetimeHandle<'_>,
                 query: &mut PacketSwitchQuery<'_>| {
                    if event.status == ClientStatusCommand::RequestStats {
                        return Ok(());
                    }

                    let client = event.client.entity_view(query.world);

                    client.get::<(
                        &ConnectionId,
                        &mut Health,
                        &mut Pose,
                        &Uuid,
                        &Position,
                        &Yaw,
                        &Pitch,
                        &Xp,
                        &Flight,
                        &FlyingSpeed,
                    )>(
                        |(
                            connection,
                            health,
                            pose,
                            uuid,
                            position,
                            yaw,
                            pitch,
                            xp,
                            flight,
                            flying_speed,
                        )| {
                            health.heal(20.);

                            *pose = Pose::Standing;
                            client.modified::<Pose>(); // this is so observers detect the change

                            let pkt_health = play::HealthUpdateS2c {
                                health: health.abs(),
                                food: VarInt(20),
                                food_saturation: 5.0,
                            };

                            let pkt_respawn = play::PlayerRespawnS2c {
                                dimension_type_name: ident!("minecraft:overworld").into(),
                                dimension_name: ident!("minecraft:overworld").into(),
                                hashed_seed: 0,
                                game_mode: GameMode::Survival,
                                previous_game_mode: OptGameMode::default(),
                                is_debug: false,
                                is_flat: false,
                                copy_metadata: false,
                                last_death_location: Option::from(GlobalPos {
                                    dimension_name: ident!("minecraft:overworld").into(),
                                    position: BlockPos::from(position.as_dvec3()),
                                }),
                                portal_cooldown: VarInt::default(),
                            };

                            let pkt_xp = play::ExperienceBarUpdateS2c {
                                bar: xp.get_visual().prop,
                                level: VarInt(i32::from(xp.get_visual().level)),
                                total_xp: VarInt::default(),
                            };

                            let pkt_abilities = PlayerAbilitiesS2c {
                                flags: PlayerAbilitiesFlags::default()
                                    .with_flying(flight.allow)
                                    .with_allow_flying(flight.is_flying),
                                flying_speed: flying_speed.speed,
                                fov_modifier: 0.0,
                            };

                            let pkt_add_player = play::PlayerSpawnS2c {
                                entity_id: VarInt(client.minecraft_id()),
                                player_uuid: uuid.0,
                                position: position.as_dvec3(),
                                yaw: ByteAngle::from_degrees(**yaw),
                                pitch: ByteAngle::from_degrees(**pitch),
                            };

                            let mut bundle = DataBundle::new(query.compose, query.system);
                            bundle.add_packet(&pkt_health).unwrap();
                            bundle.add_packet(&pkt_respawn).unwrap();
                            bundle.add_packet(&pkt_xp).unwrap();
                            bundle.add_packet(&pkt_abilities).unwrap();

                            bundle.unicast(*connection).unwrap();
                            query
                                .compose
                                .broadcast(&pkt_add_player, query.system)
                                .exclude(*connection)
                                .send()
                                .unwrap();
                        },
                    );

                    Ok(())
                },
            ));
        });
    }
}
