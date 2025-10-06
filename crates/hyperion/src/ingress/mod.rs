use std::borrow::Cow;

use bevy::prelude::*;
use colored::Colorize;
use hyperion_utils::EntityExt;
use serde_json::json;
use sha2::Digest;
use tracing::{error, info, warn};
use valence_protocol::{
    VarInt,
    packets::{
        handshaking::handshake_c2s::HandshakeNextState,
        login::{LoginCompressionS2c, LoginSuccessS2c},
        play::{EntitiesDestroyS2c, PlayerRemoveS2c},
        status::{QueryPongS2c, QueryResponseS2c},
    },
};

use crate::{
    InitializePlayerPosition,
    command_channel::CommandChannel,
    egress::sync_chunks::ChunkSendQueue,
    net::{Compose, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder},
    runtime::AsyncRuntime,
    simulation::{
        AiTargetable,
        ChunkPosition,
        ImmuneStatus,
        Pitch,
        Uuid,
        Velocity,
        Xp,
        Yaw,
        animation::ActiveAnimation,
        entity_kind::EntityKind,
        packet,
        // animation::ActiveAnimation,
        // blocks::Blocks,
        // handlers::PacketSwitchQuery,
        // metadata::{MetadataPrefabs, entity::Pose},
        packet_state,
        skin::PlayerSkin,
    },
    storage::SkinHandler,
    util::mojang::MojangClient,
};

pub mod decode;

pub fn process_handshake(
    mut packets: EventReader<'_, '_, packet::handshake::Handshake>,
    mut commands: Commands<'_, '_>,
) {
    for packet in packets.read() {
        let mut entity = commands.entity(packet.sender());

        entity.remove::<packet_state::Handshake>();
        match packet.next_state {
            HandshakeNextState::Status => {
                entity.insert(packet_state::Status(()));
            }
            HandshakeNextState::Login => {
                entity.insert(packet_state::Login(()));
            }
        }
    }
}

#[derive(Resource)]
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

fn process_status_request(
    mut packets: EventReader<'_, '_, packet::status::QueryRequest>,
    ping_response_data: Res<'_, ServerPingResponse>,
    compose: Res<'_, Compose>,
) {
    for packet in packets.read() {
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

        let json = serde_json::to_string_pretty(&json).expect("json serialization should succeed");

        let send = QueryResponseS2c {
            json: json.as_str().into(),
        };

        info!("sent query response: {packet:?}");
        compose
            .unicast_no_compression(&send, packet.connection_id())
            .unwrap();
    }
}

fn process_status_ping(
    mut packets: EventReader<'_, '_, packet::status::QueryPing>,
    compose: Res<'_, Compose>,
) {
    for packet in packets.read() {
        let payload = packet.payload;
        let send = QueryPongS2c { payload };
        info!("sent ping response: {send:?}");
        compose
            .unicast_no_compression(&send, packet.connection_id())
            .unwrap();
    }
}
pub fn process_login_hello(
    mut packets: EventReader<'_, '_, packet::login::LoginHello>,
    compose: Res<'_, Compose>,
    runtime: Res<'_, AsyncRuntime>,
    skins_collection: Res<'_, SkinHandler>,
    mojang: Res<'_, MojangClient>,
    command_channel: Res<'_, CommandChannel>,
    mut commands: Commands<'_, '_>,
    mut query: Query<'_, '_, &mut PacketDecoder>,
) {
    for packet in packets.read() {
        let sender = packet.sender();
        let mut decoder = query
            .get_mut(sender)
            .expect("PacketDecoder must be available for player");

        let username = &packet.username;
        let profile_id = packet.profile_id;

        // Set compression
        let global = compose.global();
        let pkt = LoginCompressionS2c {
            threshold: VarInt(global.shared.compression_threshold.0),
        };
        compose
            .unicast_no_compression(&pkt, packet.connection_id())
            .unwrap();
        decoder.set_compression(global.shared.compression_threshold);

        let uuid = profile_id.unwrap_or_else(|| offline_uuid(username));
        let uuid_s = format!("{uuid:?}").dimmed();
        info!("Starting login: {sender:?} {username} {uuid_s}");

        let pkt = LoginSuccessS2c {
            uuid,
            username: username.clone(),
            properties: Cow::default(),
        };

        compose.unicast(&pkt, packet.connection_id()).unwrap();

        let skin = if profile_id.is_some() {
            let mojang = mojang.as_ref().clone();
            let skins_collection = skins_collection.as_ref().clone();
            let command_channel = command_channel.as_ref().clone();
            runtime.spawn(async move {
                let skin = match PlayerSkin::from_uuid(uuid, &mojang, &skins_collection).await {
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

                command_channel.push(move |world: &mut World| {
                    let Ok(mut entity) = world.get_entity_mut(sender) else {
                        warn!(
                            "failed to get entity after skin has been fetched (likely because the \
                             player has already left the server)"
                        );
                        return;
                    };

                    entity.insert(skin);
                });
            });
            None
        } else {
            Some(PlayerSkin::EMPTY)
        };

        let username = username.to_string();
        commands.queue(move |world: &mut World| {
            let mut entity = world.entity_mut(sender);

            // TODO: The more specific components (such as ChunkSendQueue) should be added in a
            // separate system
            entity.remove::<packet_state::Login>().insert((
                Name::new(username.to_string()),
                ActiveAnimation::NONE,
                AiTargetable,
                ImmuneStatus::default(),
                Uuid::from(uuid),
                ChunkPosition::null(),
                ChunkSendQueue::default(),
                Yaw::default(),
                Pitch::default(),
                Velocity::default(),
                Xp::default(),
                EntityKind::Player,
            ));

            world.trigger(InitializePlayerPosition(sender));

            if let Some(skin) = skin {
                let mut entity = world.entity_mut(sender);
                entity.insert(skin);
            }
        });
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

fn remove_player_from_visibility(
    trigger: Trigger<'_, OnRemove, packet_state::Play>,
    query: Query<'_, '_, &Uuid>,
    compose: Res<'_, Compose>,
) {
    let uuid = match query.get(trigger.target()) {
        Ok(uuid) => uuid,
        Err(e) => {
            error!("failed to send player remove packet: query failed: {e}");
            return;
        }
    };

    let uuids = &[uuid.0];
    let entity_ids = [VarInt(trigger.target().minecraft_id())];

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
}

pub struct IngressPlugin;

impl Plugin for IngressPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(decode::DecodePlugin);
        app.add_systems(
            FixedUpdate,
            (
                process_handshake.after(decode::handshake),
                (process_status_request, process_status_ping).after(decode::status),
                process_login_hello.after(decode::login),
            ),
        );
        app.add_observer(remove_player_from_visibility);
        app.init_resource::<ServerPingResponse>();
    }
}
