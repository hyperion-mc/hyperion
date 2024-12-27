use hyperion::{
    flecs_ecs::{
        self,
        core::{EntityViewGet, World, WorldGet},
        macros::Component,
        prelude::Module,
    },
    net::{ConnectionId, DataBundle},
    protocol::{game_mode::OptGameMode, packets::play, ByteAngle, VarInt},
    server::{ident, GameMode},
    simulation::{
        event::ClientStatusCommand,
        metadata::{entity::Pose, living_entity::Health},
        Pitch, Position, Uuid, Xp, Yaw,
    },
    storage::GlobalEventHandlers,
};
use hyperion_utils::EntityExt;

#[derive(Component)]
pub struct RespawnModule;

impl Module for RespawnModule {
    fn module(world: &World) {
        world.get::<&mut GlobalEventHandlers>(|handlers| {
            handlers.client_status.register(|query, event| {
                if event.status == ClientStatusCommand::RequestStats {
                    return;
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
                )>(
                    |(connection, health, pose, uuid, position, yaw, pitch, xp)| {
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
                            last_death_location: None,
                            portal_cooldown: VarInt::default(),
                        };

                        let pkt_xp = play::ExperienceBarUpdateS2c {
                            bar: xp.get_visual().prop,
                            level: VarInt(i32::from(xp.get_visual().level)),
                            total_xp: VarInt::default(),
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

                        bundle.unicast(*connection).unwrap();
                        query
                            .compose
                            .broadcast(&pkt_add_player, query.system)
                            .send()
                            .unwrap();
                    },
                );
            });
        });
    }
}
