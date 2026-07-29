//! Putting a 26.2 client into the world.
//!
//! The one packet that means a player has joined is `Login`. Anything short of
//! it -- an accepted profile, a finished configuration, a connection the proxy
//! still counts -- leaves the client on "Joining world..." indefinitely while
//! every count on the server says it is online. That failure has happened here
//! before, so this module exists to make the transition into play one call with
//! one caller rather than something spread across a channel.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId,
    packets::play_login::{
        BlockPos, ClockNetworkState, CommonPlayerSpawnInfo, GameEvent, GameType, GlobalPos, Login,
        PlayerPosition, PositionMoveRotation, Relative, SetChunkCacheCenter, SetDefaultSpawnPosition,
        SetTime, Vec3,
    },
};
use hyperion_utils::EntityExt;
use tracing::{error, info};

use crate::{
    config::Config,
    egress::player_join::roster,
    net::{
        Channel, Compose, ConnectionId,
        protocol::{registries, send},
    },
    simulation::{Comms, MovementTracking, Pitch, Position, WorldTime, Yaw, gamemode},
};

/// The level this server serves. Only one, so the dimension list in [`Login`]
/// and the level key in every spawn info are the same string.
const LEVEL: &str = "minecraft:overworld";

/// `DimensionType.overworld().minY() + height` puts the overworld's sea level
/// at 63; the client uses it for fog and water ambience before it has a chunk.
const SEA_LEVEL: i32 = 63;

/// The level description both [`Login`] and a later respawn carry.
///
/// Shared rather than written out twice: a respawn whose spawn info disagrees
/// with the one the client joined on is how a client ends up rendering the
/// wrong horizon or the wrong water level, and neither is an error anywhere.
///
/// # Errors
/// Returns an error when the world has no `minecraft:overworld` dimension
/// type, which would mean [`registries`] and [`LEVEL`] have drifted apart.
pub fn spawn_info(game_type: GameType) -> anyhow::Result<CommonPlayerSpawnInfo<'static>> {
    let dimension_type = registries::DIMENSION_TYPE
        .id_of(LEVEL)
        .ok_or_else(|| anyhow::anyhow!("no dimension type named {LEVEL}"))?;

    Ok(CommonPlayerSpawnInfo {
        dimension_type,
        dimension: LEVEL,
        // The client only uses this to seed its own biome noise, and this
        // server sends biomes explicitly, so any value renders the same.
        seed: 0,
        game_type,
        previous_game_type: None,
        is_debug: false,
        is_flat: false,
        last_death_location: None,
        portal_cooldown: 0,
        sea_level: SEA_LEVEL,
    })
}

/// Tell `entity`'s client where the server thinks it is.
///
/// Sent on join and again after any `Respawn`, which leaves the client at the
/// dimension's default position rather than where it was.
///
/// # Errors
/// Returns an error when the entity has no position, or when the packet fails
/// to encode.
pub fn send_position(
    entity: EntityView<'_>,
    compose: &Compose,
    connection_id: ConnectionId,
) -> anyhow::Result<()> {
    let (position, yaw, pitch) = entity
        .try_get::<(&Position, &Yaw, &Pitch)>(|(position, yaw, pitch)| (**position, **yaw, **pitch))
        .ok_or_else(|| anyhow::anyhow!("cannot position a player that has none"))?;

    send(
        compose,
        connection_id,
        PacketId::PlayerPosition.to_raw(),
        &PlayerPosition {
            id: 1,
            change: PositionMoveRotation {
                position: Vec3 {
                    x: f64::from(position.x),
                    y: f64::from(position.y),
                    z: f64::from(position.z),
                },
                delta_movement: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                y_rot: yaw,
                x_rot: pitch,
            },
            relatives: Relative::NONE,
        },
    )
}

/// Send the join sequence and leave the client in play.
///
/// # Errors
/// Returns an error when a packet fails to encode or when the world has no
/// `minecraft:overworld` dimension type, which would mean [`registries`] and
/// the level key here have drifted apart.
pub fn enter_world(
    world: &WorldRef<'_>,
    entity: EntityView<'_>,
    compose: &Compose,
    connection_id: ConnectionId,
) -> anyhow::Result<()> {
    let position = entity
        .try_get::<&Position>(|position| **position)
        .ok_or_else(|| anyhow::anyhow!("player finished configuration without a spawn position"))?;

    let (max_players, chunk_radius, simulation_distance) = world.get::<&Config>(|config| {
        (
            config.max_players,
            i32::from(config.view_distance),
            config.simulation_distance,
        )
    });

    send(compose, connection_id, PacketId::Login.to_raw(), &Login {
        player_id: entity.minecraft_id(),
        hardcore: false,
        levels: vec![LEVEL],
        max_players,
        chunk_radius,
        simulation_distance,
        reduced_debug_info: false,
        show_death_screen: false,
        do_limited_crafting: false,
        spawn_info: spawn_info(gamemode::of(entity).to_game_type())?,
        online_mode: false,
        enforces_secure_chat: false,
    })?;

    // Before any `AddEntity`: a client refuses to build a player entity whose
    // profile id it has not already seen in the tab list, and says so only in
    // its own log.
    roster::announce(world, entity, compose, connection_id)?;

    // The client discards chunks outside the cache centre, so this has to
    // precede any terrain rather than follow it.
    let chunk = Position::from(position).to_chunk();
    let block = position.floor().as_ivec3();
    send(
        compose,
        connection_id,
        PacketId::SetChunkCacheCenter.to_raw(),
        &SetChunkCacheCenter {
            x: i32::from(chunk.x),
            z: i32::from(chunk.y),
        },
    )?;

    send(
        compose,
        connection_id,
        PacketId::SetDefaultSpawnPosition.to_raw(),
        &SetDefaultSpawnPosition {
            global_pos: GlobalPos {
                dimension: LEVEL,
                pos: BlockPos {
                    x: block.x,
                    y: block.y,
                    z: block.z,
                },
            },
            yaw: 0.0,
            pitch: 0.0,
        },
    )?;

    send_position(entity, compose, connection_id)?;

    // Without this the client renders the terrain but never dismisses the
    // "Loading terrain..." screen, because `LEVEL_CHUNKS_LOAD_START` is what
    // `ClientPacketListener` waits on to hand control to the player.
    //
    // Vanilla sends it after the first chunk batch. hyperion streams chunks
    // from `egress::sync_chunks` on the ticks after this one, and there is no
    // point in the join path that knows when the first batch has landed, so it
    // goes here. The visible difference is that the world fades in from empty
    // rather than appearing complete.
    send(
        compose,
        connection_id,
        PacketId::GameEvent.to_raw(),
        &GameEvent {
            event: GameEvent::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        },
    )?;

    // Freeze the daylight cycle. 26.2 makes the client advance the sun itself
    // from the overworld clock's `rate`; with no `SetTime` at all the client
    // never learns a rate and free-runs its own cycle, so the sky drifts. One
    // `SetTime` with the overworld clock at `rate` 0.0 pins the sun -- the same
    // bytes a paused clock serialises to (`ServerClockManager`) -- and there is
    // no per-tick resend. `game_time` stays the real world age; only the day
    // time is frozen. The clock's network id comes from the same `world_clock`
    // registry the client was sent in configuration, so the two cannot drift.
    let day_time = world.get::<&WorldTime>(|world_time| world_time.day_time);
    let overworld_clock = registries::WORLD_CLOCK
        .id_of("minecraft:overworld")
        .ok_or_else(|| anyhow::anyhow!("world_clock registry has no minecraft:overworld"))?;
    send(
        compose,
        connection_id,
        PacketId::SetTime.to_raw(),
        &SetTime {
            game_time: compose.global().tick,
            clock_updates: vec![(
                overworld_clock,
                ClockNetworkState::frozen(day_time),
            )],
        },
    )?;

    // The movement handler reads this on every position packet and does not
    // check for it first, so a player without it aborts the process the moment
    // they walk. It is seeded with the position the client was just told to
    // spawn at, not with `default()`: the handler treats a large step from
    // `last_tick_position` as a cheat and teleports the player back to it, so a
    // zeroed tracker drags a joining player to the origin one tick later.
    entity.set(MovementTracking {
        last_tick_position: position,
        was_on_ground: true,
        ..MovementTracking::default()
    });

    // The player is now visible to other players through its own packet
    // channel, and may receive broadcasts.
    entity.add(id::<Channel>());
    compose.io_buf().set_receive_broadcasts(connection_id);

    info!("{} joined the world", entity.id());

    Ok(())
}

/// Applies skins as they arrive from the Mojang session server.
///
/// On 763 the arrival of a skin is what triggers the join. Here the join is
/// driven by the client's own `finish_configuration`, so a skin that arrives
/// late only changes how the player looks, and a skin that never arrives cannot
/// keep anyone out of the world.
#[derive(Component)]
pub struct JoinModule;

impl Module for JoinModule {
    fn module(world: &World) {
        system!("apply_skins", world, &Comms)
            .kind(id::<flecs::pipeline::PreUpdate>())
            .each_iter(|it, _, comms| {
                let world = it.world();

                while let Ok(Some((entity, skin))) = comms.skins_rx.try_recv() {
                    if !world.is_alive(entity) {
                        continue;
                    }
                    // `wear` and not `set`, because this fires after the join
                    // and the overwhelmingly common answer from the session
                    // server is the empty skin the player already publishes.
                    // A plain write would republish the profile of every
                    // client on the server a second after it joined.
                    roster::wear(world.entity_from_id(entity), skin);
                }
            });

        world
            .observer_named::<flecs::OnRemove, ()>("log_leave_without_uuid")
            .with_enum(crate::simulation::PacketState::Play)
            .each_entity(|entity, ()| {
                if entity.try_get::<&crate::simulation::Uuid>(|_| ()).is_none() {
                    error!("a player left play state without a uuid");
                }
            });
    }
}
