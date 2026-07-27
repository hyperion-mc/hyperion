use std::{borrow::Cow, sync::Arc};

use colored::Colorize;
use flecs_ecs::prelude::*;
use hyperion_utils::EntityExt;
use serde_json::json;
use sha2::Digest;
use tracing::{error, info, info_span, warn};
use valence_protocol::{
    Bounded, VarInt,
    packets::{
        handshaking::handshake_c2s::HandshakeNextState,
        login::{LoginCompressionS2c, LoginSuccessS2c},
        play::{EntitiesDestroyS2c, PlayerRemoveS2c},
        status::{QueryPongS2c, QueryResponseS2c},
    },
};

use crate::{
    Prev, Shutdown,
    command_channel::CommandChannel,
    egress::sync_chunks::ChunkSendQueue,
    ingress::decode::{DecodeModule, queues},
    net::{Compose, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder},
    runtime::AsyncRuntime,
    simulation::{
        AiTargetable, ChunkPosition, ConfirmBlockSequences, IgnMap, ImmuneStatus, Name, Player,
        Uuid, Velocity, Xp,
        animation::ActiveAnimation,
        entity_kind::EntityKind,
        metadata::MetadataPrefabs,
        packet_state,
        skin::PlayerSkin,
    },
    storage::SkinHandler,
    util::mojang::MojangClient,
};

pub mod decode;

/// This marks players who have already been disconnected and about to be destructed. This component should not be
/// added to an entity to disconnect a player. Use [`crate::net::IoBuf::shutdown`] instead.
#[derive(Component, Debug)]
pub struct PendingRemove;

/// The data sent to clients which ping the server without logging in.
#[derive(Component)]
pub struct ServerPingResponse {
    pub description: String,
    pub max_players: u32,
}

impl Default for ServerPingResponse {
    fn default() -> Self {
        Self {
            description: String::from(
                "Getting 10k Players to PvP at Once on a Minecraft Server to Break the Guinness \
                 World Record",
            ),
            max_players: 12_000,
        }
    }
}

/// Get a [`uuid::Uuid`] based on the given user's name.
fn offline_uuid(username: &str) -> uuid::Uuid {
    let digest = sha2::Sha256::digest(username);
    let digest: [u8; 32] = digest.into();
    let (&digest, ..) = digest.split_array_ref::<16>();

    // todo: I have no idea which way we should go (be or le)
    let digest = u128::from_be_bytes(digest);
    uuid::Uuid::from_u128(digest)
}

fn process_handshake(world: &World) {
    world
        .system_named::<&mut queues::handshake>("process_handshake")
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(|it, _, queue| {
            let world = it.world();

            for packet in std::mem::take(&mut queue.Handshake) {
                let entity = world.entity_from_id(packet.sender());

                entity.remove(id::<packet_state::Handshake>());

                // todo: check version is correct
                match packet.next_state {
                    HandshakeNextState::Status => {
                        entity.add(id::<packet_state::Status>());
                    }
                    HandshakeNextState::Login => {
                        entity.add(id::<packet_state::Login>());
                    }
                }
            }
        });
}

fn process_status_request(world: &World) {
    world
        .system_named::<(&mut queues::status, &ServerPingResponse, &Compose)>(
            "process_status_request",
        )
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(|(queue, ping_response_data, compose)| {
            for packet in std::mem::take(&mut queue.QueryRequest) {
                // let img_bytes = include_bytes!("data/hyperion.png");

                // let favicon = general_purpose::STANDARD.encode(img_bytes);
                // let favicon = format!("data:image/png;base64,{favicon}");

                let online = compose
                    .global()
                    .player_count
                    .load(std::sync::atomic::Ordering::Relaxed);

                // https://wiki.vg/Server_List_Ping#Response
                let json = json!({
                    "version": {
                        "name": MINECRAFT_VERSION,
                        "protocol": PROTOCOL_VERSION,
                    },
                    "players": {
                        "online": online,
                        "max": ping_response_data.max_players,
                        "sample": [],
                    },
                    "description": ping_response_data.description,
                    // "favicon": favicon,
                });

                let json =
                    serde_json::to_string_pretty(&json).expect("json serialization should succeed");

                let send = QueryResponseS2c {
                    json: json.as_str(),
                };

                info!("sent query response: {packet:?}");

                if let Err(e) = compose.unicast_no_compression(&send, packet.connection_id()) {
                    error!("failed to send query response: {e}");
                }
            }
        });
}

fn process_status_ping(world: &World) {
    world
        .system_named::<(&mut queues::status, &Compose)>("process_status_ping")
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(|(queue, compose)| {
            for packet in std::mem::take(&mut queue.QueryPing) {
                let payload = packet.payload;
                let send = QueryPongS2c { payload };
                info!("sent ping response: {send:?}");

                if let Err(e) = compose.unicast_no_compression(&send, packet.connection_id()) {
                    error!("failed to send ping response: {e}");
                }
            }
        });
}

fn process_login_hello(world: &World) {
    world
        .system_named::<(
            &mut queues::login,
            &Compose,
            &AsyncRuntime,
            &SkinHandler,
            &MojangClient,
            &CommandChannel,
            &IgnMap,
        )>("process_login_hello")
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(
            |it,
             _,
             (queue, compose, runtime, skins_collection, mojang, command_channel, ign_map)| {
                let world = it.world();

                for packet in std::mem::take(&mut queue.LoginHello) {
                    let sender = packet.sender();
                    let connection_id = packet.connection_id();
                    let entity = world.entity_from_id(sender);

                    let username: Arc<str> = Arc::from(packet.username.0);
                    let profile_id = packet.profile_id;

                    // Set compression
                    let global = compose.global();
                    let pkt = LoginCompressionS2c {
                        threshold: VarInt(global.shared.compression_threshold.0),
                    };

                    if let Err(e) = compose.unicast_no_compression(&pkt, connection_id) {
                        error!("failed to send login compression packet: {e}");
                        compose.io_buf().shutdown(connection_id);
                        continue;
                    }

                    entity.get::<&mut PacketDecoder>(|decoder| {
                        decoder.set_compression(global.shared.compression_threshold);
                    });

                    let uuid = profile_id.unwrap_or_else(|| offline_uuid(&username));
                    let uuid_s = format!("{uuid:?}").dimmed();
                    info!("Starting login: {sender:?} {username} {uuid_s}");

                    let pkt = LoginSuccessS2c {
                        uuid,
                        username: Bounded(&username),
                        properties: Cow::default(),
                    };

                    if let Err(e) = compose.unicast(&pkt, connection_id) {
                        error!("failed to send login success packet: {e}");
                        compose.io_buf().shutdown(connection_id);
                        continue;
                    }

                    // Skins for players with a real Mojang profile are fetched asynchronously and
                    // applied through the command channel once they arrive.
                    let skin = if profile_id.is_some() {
                        let mojang = mojang.clone();
                        let skins_collection = skins_collection.clone();
                        let command_channel = command_channel.clone();

                        runtime.spawn(async move {
                            let skin =
                                match PlayerSkin::from_uuid(uuid, &mojang, &skins_collection).await
                                {
                                    Ok(Some(skin)) => skin,
                                    Err(e) => {
                                        error!("failed to get skin {e}. Using empty skin");
                                        PlayerSkin::EMPTY
                                    }
                                    Ok(None) => {
                                        error!("failed to get skin. Using empty skin");
                                        PlayerSkin::EMPTY
                                    }
                                };

                            command_channel.push(move |world: &World| {
                                let entity = world.entity_from_id(sender);

                                if !entity.is_alive() {
                                    warn!(
                                        "failed to get entity after skin has been fetched (likely \
                                         because the player has already left the server)"
                                    );
                                    return;
                                }

                                entity.set(skin);
                            });
                        });

                        None
                    } else {
                        Some(PlayerSkin::EMPTY)
                    };

                    ign_map.insert(username.clone(), sender, &world);

                    // TODO: The more specific components (such as ChunkSendQueue) should be added
                    // in a separate system
                    world.get::<&MetadataPrefabs>(|prefabs| {
                        entity
                            .is_a(prefabs.player_base)
                            .set(Name::from(username.clone()))
                            .add(id::<AiTargetable>())
                            .set(ImmuneStatus::default())
                            .set(Uuid::from(uuid))
                            .add(id::<Xp>())
                            .set_pair::<Prev, _>(Xp::default())
                            .add(id::<ChunkSendQueue>())
                            .add(id::<Velocity>())
                            .set(ChunkPosition::null())
                            .set(ActiveAnimation::NONE)
                            .set(hyperion_inventory::PlayerInventory::default())
                            .set(ConfirmBlockSequences::default())
                            .add_enum(EntityKind::Player)
                            .add(id::<Player>());
                    });

                    if let Some(skin) = skin {
                        entity.set(skin);
                    }

                    entity.remove(id::<packet_state::Login>());
                    entity.add(id::<packet_state::Play>());

                    compose.io_buf().set_receive_broadcasts(connection_id);
                }
            },
        );
}

fn remove_player_from_visibility(world: &World) {
    world
        .observer::<flecs::OnRemove, ()>()
        .with(id::<packet_state::Play>())
        .each_entity(|entity, ()| {
            let world = entity.world();

            let Some(uuid) = entity.try_get::<&Uuid>(|uuid| uuid.0) else {
                error!("failed to send player remove packet: player has no uuid");
                return;
            };

            world.get::<&Compose>(|compose| {
                let uuids = &[uuid];
                let entity_ids = [VarInt(entity.id().minecraft_id())];

                // destroy
                let pkt = EntitiesDestroyS2c {
                    entity_ids: Cow::Borrowed(&entity_ids),
                };

                if let Err(e) = compose.broadcast(&pkt).send() {
                    error!("failed to send player remove packet: {e}");
                    return;
                }

                let pkt = PlayerRemoveS2c {
                    uuids: Cow::Borrowed(uuids),
                };

                if let Err(e) = compose.broadcast(&pkt).send() {
                    error!("failed to send player remove packet: {e}");
                }
            });
        });
}

#[derive(Component)]
pub struct IngressModule;

impl Module for IngressModule {
    fn module(world: &World) {
        world.component::<PendingRemove>();
        world
            .component::<ServerPingResponse>()
            .add_trait::<flecs::Singleton>();
        world.set(ServerPingResponse::default());

        world.import::<DecodeModule>();

        system!("shutdown", world, &Shutdown)
            .kind(id::<flecs::pipeline::OnLoad>())
            .each_iter(|it, _, shutdown| {
                let world = it.world();
                if shutdown.value.load(std::sync::atomic::Ordering::Relaxed) {
                    info!("shutting down");
                    world.quit();
                }
            });

        system!("update_ign_map", world, &mut IgnMap)
            .kind(id::<flecs::pipeline::OnLoad>())
            .each_iter(|_, _, ign_map| {
                let span = info_span!("update_ign_map");
                let _enter = span.enter();
                ign_map.update();
            });

        process_handshake(world);
        process_status_request(world);
        process_status_ping(world);
        process_login_hello(world);

        remove_player_from_visibility(world);

        world
            .system_named::<()>("remove_player")
            .kind(id::<flecs::pipeline::PostLoad>())
            .with(id::<PendingRemove>())
            .each_entity(|entity, ()| {
                entity.destruct();
            });
    }
}
