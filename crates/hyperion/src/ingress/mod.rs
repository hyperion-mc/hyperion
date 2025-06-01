use std::{borrow::Cow, sync::Arc};

use anyhow::Context;
use bevy::prelude::*;
use colored::Colorize;
use hyperion_packet_macros::*;
use hyperion_utils::EntityExt;
use serde_json::json;
use sha2::Digest;
use tracing::{error, info, info_span, trace, warn};
use valence_protocol::{
    Bounded, DecodeBytes, Packet, VarInt, packets,
    packets::{
        handshaking, handshaking::handshake_c2s::HandshakeNextState, login,
        login::LoginCompressionS2c, play, status,
    },
};
use valence_text::IntoText;

use crate::{
    Prev,
    Shutdown,
    command_channel::CommandChannel,
    // egress::sync_chunks::ChunkSendQueue,
    net::{
        Compose, ConnectionId, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder,
        decoder::BorrowedPacketFrame, packet_channel,
    },
    runtime::AsyncRuntime,
    simulation::{
        AiTargetable,
        ChunkPosition,
        ConfirmBlockSequences,
        EntitySize,
        IgnMap,
        ImmuneStatus,
        Name,
        Pitch,
        Player,
        Position,
        StreamLookup,
        Uuid,
        Velocity,
        Xp,
        Yaw,
        // animation::ActiveAnimation,
        // blocks::Blocks,
        // handlers::PacketSwitchQuery,
        // metadata::{MetadataPrefabs, entity::Pose},
        packet::{HandshakePacket, LoginPacket, PlayPacket, StatusPacket},
        packet_state,
        skin::PlayerSkin,
    },
    storage::SkinHandler,
    util::mojang::MojangClient,
};

/// This event is sent for players who have already been disconnected and about to be destructed. Packets
/// cannot be sent to these players because they have already been disconnected.
///
/// This event cannot be sent to disconnect a player. Use [`crate::net::IoBuf::shutdown`] instead.
#[derive(Event)]
pub struct Disconnect(pub(crate) ());

pub fn process_handshake(trigger: Trigger<'_, HandshakePacket>, mut commands: Commands<'_, '_>) {
    let HandshakePacket::Handshake(handshake) = trigger.event();
    let mut entity = commands.entity(trigger.target());

    entity.remove::<packet_state::Handshake>();
    match handshake.next_state {
        HandshakeNextState::Status => {
            entity.insert(packet_state::Status(()));
        }
        HandshakeNextState::Login => {
            entity.insert(packet_state::Login(()));
        }
    }
}

fn process_status(
    trigger: Trigger<'_, StatusPacket>,
    query: Query<'_, '_, &ConnectionId>,
    compose: Res<'_, Compose>,
) {
    let connection_id = *query
        .get(trigger.target())
        .expect("ConnectionId must be available for player");

    match trigger.event() {
        StatusPacket::QueryRequest(query_request) => {
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
                    "max": 12_000,
                    "sample": [],
                },
                "description": "Getting 10k Players to PvP at Once on a Minecraft Server to Break the Guinness World Record",
                // "favicon": favicon,
            });

            let json =
                serde_json::to_string_pretty(&json).expect("json serialization should succeed");

            let send = packets::status::QueryResponseS2c {
                json: json.as_str().into(),
            };

            info!("sent query response: {query_request:?}");
            compose
                .unicast_no_compression(&send, connection_id)
                .unwrap();
        }
        StatusPacket::QueryPing(query_ping) => {
            let payload = query_ping.payload;
            let send = packets::status::QueryPongS2c { payload };
            info!("sent ping response: {send:?}");
            compose
                .unicast_no_compression(&send, connection_id)
                .unwrap();
        }
    }
}

pub fn process_login(
    trigger: Trigger<'_, LoginPacket>,
    compose: Res<'_, Compose>,
    runtime: Res<'_, AsyncRuntime>,
    skins_collection: Res<'_, SkinHandler>,
    mojang: Res<'_, MojangClient>,
    command_channel: Res<'_, CommandChannel>,
    mut commands: Commands<'_, '_>,
    mut query: Query<'_, '_, (&ConnectionId, &mut PacketDecoder)>,
) {
    let entity_id = trigger.target();
    let (&connection_id, mut decoder) = query
        .get_mut(entity_id)
        .expect("ConnectionId and PacketDecoder must be available for player");
    let mojang = mojang.into_inner().clone();
    let skins_collection = skins_collection.into_inner().clone();
    let command_channel = command_channel.into_inner().clone();

    let LoginPacket::LoginHello(login) = trigger.event() else {
        error!("expected LoginHello during login state");
        compose.io_buf().shutdown(connection_id);
        return;
    };

    let username = Arc::from(&*login.username.0);
    let profile_id = login.profile_id;

    // Set compression
    let global = compose.global();
    let pkt = LoginCompressionS2c {
        threshold: VarInt(global.shared.compression_threshold.0),
    };
    compose.unicast_no_compression(&pkt, connection_id).unwrap();
    decoder.set_compression(global.shared.compression_threshold);

    let uuid = profile_id.unwrap_or_else(|| offline_uuid(&username));
    let uuid_s = format!("{uuid:?}").dimmed();
    info!("Starting login: {username} {uuid_s}");

    let pkt = login::LoginSuccessS2c {
        uuid,
        username: login.username.clone(),
        properties: Cow::default(),
    };

    compose.unicast(&pkt, connection_id).unwrap();

    let skin = if profile_id.is_some() {
        let mojang = mojang.clone();
        let skins_collection = skins_collection.clone();
        let command_channel = command_channel.clone();
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
                let Ok(mut entity) = world.get_entity_mut(entity_id) else {
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

    let mut entity = commands.entity(entity_id);
    entity
        .remove::<packet_state::Login>()
        .insert(Name::from(username))
        .insert(AiTargetable)
        .insert(ImmuneStatus::default())
        .insert(Uuid::from(uuid))
        .insert(ChunkPosition::null())
        .insert(Yaw::default())
        .insert(Pitch::default())
        .insert(packet_state::Play(()));
    if let Some(skin) = skin {
        entity.insert(skin);
    }

    compose.io_buf().set_receive_broadcasts(connection_id);
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

fn try_next_frame(
    compose: &Compose,
    connection_id: ConnectionId,
    decoder: &mut PacketDecoder,
    receiver: &mut packet_channel::Receiver,
) -> Option<BorrowedPacketFrame> {
    let raw_packet = receiver.try_recv()?;
    let bump = compose.bump();
    match decoder.try_next_packet(bump, &raw_packet) {
        Ok(Some(packet)) => Some(packet),
        Ok(None) => None,
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

fn try_decode<P: Packet + DecodeBytes>(
    frame: BorrowedPacketFrame,
    compose: &Compose,
    connection_id: ConnectionId,
) -> Option<P> {
    match frame.decode() {
        Ok(packet) => Some(packet),
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

fn decode_handshake_packets(
    query: Query<
        '_,
        '_,
        (
            Entity,
            &ConnectionId,
            &mut PacketDecoder,
            &mut packet_channel::Receiver,
        ),
        With<packet_state::Handshake>,
    >,
    compose: Res<'_, Compose>,
    mut commands: Commands<'_, '_>,
) {
    let compose = compose.into_inner();
    for (entity_id, &connection_id, decoder, receiver) in query {
        let Some(frame) = try_next_frame(
            compose,
            connection_id,
            decoder.into_inner(),
            receiver.into_inner(),
        ) else {
            continue;
        };

        let packet = match frame.id {
            handshaking::HandshakeC2s::ID => {
                try_decode(frame, compose, connection_id).map(HandshakePacket::Handshake)
            }
            _ => {
                error!("unknown handshake packet id: {}", frame.id);
                compose.io_buf().shutdown(connection_id);
                None
            }
        };

        if let Some(packet) = packet {
            commands.trigger_targets(packet, entity_id);
        }
    }
}

fn decode_status_packets(
    query: Query<
        '_,
        '_,
        (
            Entity,
            &ConnectionId,
            &mut PacketDecoder,
            &mut packet_channel::Receiver,
        ),
        With<packet_state::Status>,
    >,
    compose: Res<'_, Compose>,
    mut commands: Commands<'_, '_>,
) {
    let compose = compose.into_inner();
    for (entity_id, &connection_id, decoder, receiver) in query {
        let Some(frame) = try_next_frame(
            compose,
            connection_id,
            decoder.into_inner(),
            receiver.into_inner(),
        ) else {
            continue;
        };

        let packet = match frame.id {
            status::QueryPingC2s::ID => {
                try_decode(frame, compose, connection_id).map(StatusPacket::QueryPing)
            }
            status::QueryRequestC2s::ID => {
                try_decode(frame, compose, connection_id).map(StatusPacket::QueryRequest)
            }
            _ => {
                error!("unknown status packet id: {}", frame.id);
                compose.io_buf().shutdown(connection_id);
                None
            }
        };

        if let Some(packet) = packet {
            commands.trigger_targets(packet, entity_id);
        }
    }
}

fn decode_login_packets(
    query: Query<
        '_,
        '_,
        (
            Entity,
            &ConnectionId,
            &mut PacketDecoder,
            &mut packet_channel::Receiver,
        ),
        With<packet_state::Login>,
    >,
    compose: Res<'_, Compose>,
    mut commands: Commands<'_, '_>,
) {
    let compose = compose.into_inner();
    for (entity_id, &connection_id, decoder, receiver) in query {
        let Some(frame) = try_next_frame(
            compose,
            connection_id,
            decoder.into_inner(),
            receiver.into_inner(),
        ) else {
            continue;
        };

        let packet = match frame.id {
            login::LoginHelloC2s::ID => {
                try_decode(frame, compose, connection_id).map(LoginPacket::LoginHello)
            }
            login::LoginKeyC2s::ID => {
                try_decode(frame, compose, connection_id).map(LoginPacket::LoginKey)
            }
            login::LoginQueryResponseC2s::ID => {
                try_decode(frame, compose, connection_id).map(LoginPacket::LoginQueryResponse)
            }
            _ => {
                error!("unknown login packet id: {}", frame.id);
                compose.io_buf().shutdown(connection_id);
                None
            }
        };

        if let Some(packet) = packet {
            commands.trigger_targets(packet, entity_id);
        }
    }
}

fn decode_play_packets(
    query: Query<
        '_,
        '_,
        (
            Entity,
            &ConnectionId,
            &mut PacketDecoder,
            &mut packet_channel::Receiver,
        ),
        With<packet_state::Play>,
    >,
    compose: Res<'_, Compose>,
    mut commands: Commands<'_, '_>,
) {
    let compose = compose.into_inner();
    for (entity_id, &connection_id, decoder, receiver) in query {
        let decoder = decoder.into_inner();
        let receiver = receiver.into_inner();
        while let Some(frame) = try_next_frame(compose, connection_id, decoder, receiver) {
            let packet = match frame.id {
                packets::play::AdvancementTabC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::AdvancementTab)
                }
                packets::play::BoatPaddleStateC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::BoatPaddleState)
                }
                packets::play::BookUpdateC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::BookUpdate)
                }
                packets::play::ButtonClickC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ButtonClick)
                }
                packets::play::ChatMessageC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ChatMessage)
                }
                packets::play::ClickSlotC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ClickSlot)
                }
                packets::play::ClientCommandC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ClientCommand)
                }
                packets::play::ClientSettingsC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ClientSettings)
                }
                packets::play::ClientStatusC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ClientStatus)
                }
                packets::play::CloseHandledScreenC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::CloseHandledScreen)
                }
                packets::play::CommandExecutionC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::CommandExecution)
                }
                packets::play::CraftRequestC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::CraftRequest)
                }
                packets::play::CreativeInventoryActionC2s::ID => {
                    try_decode(frame, compose, connection_id)
                        .map(PlayPacket::CreativeInventoryAction)
                }
                packets::play::CustomPayloadC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::CustomPayload)
                }
                packets::play::FullC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::Full)
                }
                packets::play::HandSwingC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::HandSwing)
                }
                packets::play::JigsawGeneratingC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::JigsawGenerating)
                }
                packets::play::KeepAliveC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::KeepAlive)
                }
                packets::play::LookAndOnGroundC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::LookAndOnGround)
                }
                packets::play::MessageAcknowledgmentC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::MessageAcknowledgment)
                }
                packets::play::OnGroundOnlyC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::OnGroundOnly)
                }
                packets::play::PickFromInventoryC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PickFromInventory)
                }
                packets::play::PlayPongC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayPong)
                }
                packets::play::PlayerActionC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerAction)
                }
                packets::play::PlayerInputC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerInput)
                }
                packets::play::PlayerInteractBlockC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerInteractBlock)
                }
                packets::play::PlayerInteractEntityC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerInteractEntity)
                }
                packets::play::PlayerInteractItemC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerInteractItem)
                }
                packets::play::PlayerSessionC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PlayerSession)
                }
                packets::play::PositionAndOnGroundC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::PositionAndOnGround)
                }
                packets::play::QueryBlockNbtC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::QueryBlockNbt)
                }
                packets::play::QueryEntityNbtC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::QueryEntityNbt)
                }
                packets::play::RecipeBookDataC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::RecipeBookData)
                }
                packets::play::RecipeCategoryOptionsC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::RecipeCategoryOptions)
                }
                packets::play::RenameItemC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::RenameItem)
                }
                packets::play::RequestCommandCompletionsC2s::ID => {
                    try_decode(frame, compose, connection_id)
                        .map(PlayPacket::RequestCommandCompletions)
                }
                packets::play::ResourcePackStatusC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::ResourcePackStatus)
                }
                packets::play::SelectMerchantTradeC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::SelectMerchantTrade)
                }
                packets::play::SpectatorTeleportC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::SpectatorTeleport)
                }
                packets::play::TeleportConfirmC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::TeleportConfirm)
                }
                packets::play::UpdateBeaconC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateBeacon)
                }
                packets::play::UpdateCommandBlockC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateCommandBlock)
                }
                packets::play::UpdateCommandBlockMinecartC2s::ID => {
                    try_decode(frame, compose, connection_id)
                        .map(PlayPacket::UpdateCommandBlockMinecart)
                }
                packets::play::UpdateDifficultyC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateDifficulty)
                }
                packets::play::UpdateDifficultyLockC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateDifficultyLock)
                }
                packets::play::UpdateJigsawC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateJigsaw)
                }
                packets::play::UpdatePlayerAbilitiesC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdatePlayerAbilities)
                }
                packets::play::UpdateSelectedSlotC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateSelectedSlot)
                }
                packets::play::UpdateSignC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateSign)
                }
                packets::play::UpdateStructureBlockC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::UpdateStructureBlock)
                }
                packets::play::VehicleMoveC2s::ID => {
                    try_decode(frame, compose, connection_id).map(PlayPacket::VehicleMove)
                }
                _ => {
                    error!("unknown play packet id: {}", frame.id);
                    compose.io_buf().shutdown(connection_id);
                    None
                }
            };

            if let Some(packet) = packet {
                commands.trigger_targets(packet, entity_id);
            }
        }
    }
}

pub struct IngressPlugin;

impl Plugin for IngressPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<Disconnect>();
        app.add_systems(
            FixedUpdate,
            (
                decode_handshake_packets,
                decode_status_packets,
                decode_login_packets,
                decode_play_packets,
            ),
        );
        app.add_observer(process_handshake);
        app.add_observer(process_status);
        app.add_observer(process_login);

        //        system!(
        //            "remove_player_from_visibility",
        //            world,
        //            &Uuid,
        //            &Compose($),
        //        )
        //        .kind(id::<flecs::pipeline::PostLoad>())
        //        .with(id::<PendingRemove>())
        //        .each_iter(move |it, row, (uuid, compose)| {
        //            let system = it.system();
        //            let entity = it.entity(row).expect("row must be in bounds");
        //            let uuids = &[uuid.0];
        //            let entity_ids = [VarInt(entity.minecraft_id())];
        //
        //            // destroy
        //            let pkt = play::EntitiesDestroyS2c {
        //                entity_ids: Cow::Borrowed(&entity_ids),
        //            };
        //
        //            if let Err(e) = compose.broadcast(&pkt, system).send() {
        //                error!("failed to send player remove packet: {e}");
        //                return;
        //            }
        //
        //            let pkt = play::PlayerRemoveS2c {
        //                uuids: Cow::Borrowed(uuids),
        //            };
        //
        //            if let Err(e) = compose.broadcast(&pkt, system).send() {
        //                error!("failed to send player remove packet: {e}");
        //            }
        //        });
        //
        //        world
        //            .system_named::<()>("remove_player")
        //            .kind(id::<flecs::pipeline::PostLoad>())
        //            .with(id::<&PendingRemove>())
        //            .tracing_each_entity(info_span!("remove_player"), |entity, ()| {
        //                entity.destruct();
        //            });
        //
    }
}
