use bevy::prelude::*;
use hyperion::{
    ingress,
    net::{Compose, DataBundle},
    simulation::{
        metadata::{entity::Pose, living_entity::Health},
        packet::play,
        Flight, FlyingSpeed, Pitch, Position, Uuid, Xp, Yaw,
    },
};
use tracing::error;
use valence_protocol::{
    game_mode::OptGameMode,
    packets::play::{
        player_abilities_s2c::{PlayerAbilitiesFlags, PlayerAbilitiesS2c},
        ClientStatusC2s, ExperienceBarUpdateS2c, HealthUpdateS2c, PlayerRespawnS2c, PlayerSpawnS2c,
    },
    BlockPos, ByteAngle, GlobalPos, VarInt,
};
use valence_server::{ident, GameMode};

fn handle_respawn(
    mut packets: EventReader<'_, '_, play::ClientStatus>,
    mut query: Query<
        '_,
        '_,
        (
            &mut Health,
            &mut Pose,
            &Uuid,
            &Position,
            &Yaw,
            &Pitch,
            &Xp,
            &Flight,
            &FlyingSpeed,
        ),
    >,
    compose: Res<'_, Compose>,
) {
    for packet in packets.read() {
        if !matches!(**packet, ClientStatusC2s::PerformRespawn) {
            continue;
        }

        let (mut health, mut pose, uuid, position, yaw, pitch, xp, flight, flying_speed) =
            match query.get_mut(packet.sender()) {
                Ok(data) => data,
                Err(e) => {
                    error!("failed to handle respawn: query failed: {e}");
                    continue;
                }
            };

        health.heal(20.);

        *pose = Pose::Standing;

        let pkt_health = HealthUpdateS2c {
            health: health.abs(),
            food: VarInt(20),
            food_saturation: 5.0,
        };

        let pkt_respawn = PlayerRespawnS2c {
            dimension_type_name: ident!("minecraft:overworld"),
            dimension_name: ident!("minecraft:overworld"),
            hashed_seed: 0,
            game_mode: GameMode::Survival,
            previous_game_mode: OptGameMode::default(),
            is_debug: false,
            is_flat: false,
            copy_metadata: false,
            last_death_location: Option::from(GlobalPos {
                dimension_name: ident!("minecraft:overworld"),
                position: BlockPos::from(position.as_dvec3()),
            }),
            portal_cooldown: VarInt::default(),
        };

        let pkt_xp = ExperienceBarUpdateS2c {
            bar: xp.get_visual().prop,
            level: VarInt(i32::from(xp.get_visual().level)),
            total_xp: VarInt::default(),
        };

        let pkt_abilities = PlayerAbilitiesS2c {
            flags: PlayerAbilitiesFlags::default()
                .with_flying(flight.is_flying)
                .with_allow_flying(flight.allow),
            flying_speed: flying_speed.speed,
            fov_modifier: 0.0,
        };

        let mut bundle = DataBundle::new(&compose);
        bundle.add_packet(&pkt_health).unwrap();
        bundle.add_packet(&pkt_respawn).unwrap();
        bundle.add_packet(&pkt_xp).unwrap();
        bundle.add_packet(&pkt_abilities).unwrap();
        bundle.unicast(packet.connection_id()).unwrap();

        let pkt_add_player = PlayerSpawnS2c {
            entity_id: VarInt(packet.minecraft_id()),
            player_uuid: uuid.0,
            position: position.as_dvec3(),
            yaw: ByteAngle::from_degrees(**yaw),
            pitch: ByteAngle::from_degrees(**pitch),
        };

        compose
            .broadcast(&pkt_add_player)
            .exclude(packet.connection_id())
            .send()
            .unwrap();
    }
}

pub struct RespawnPlugin;

impl Plugin for RespawnPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, handle_respawn.after(ingress::decode::play));
    }
}
