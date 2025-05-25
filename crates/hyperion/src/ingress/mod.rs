use std::{borrow::Cow, sync::Arc};

use anyhow::Context;
use bevy::prelude::*;
use colored::Colorize;
use hyperion_utils::EntityExt;
use serde_json::json;
use sha2::Digest;
use tracing::{error, info, info_span, trace, warn};
use valence_protocol::{
    Bounded, Decode, Packet, VarInt, packets,
    packets::{
        handshaking::handshake_c2s::HandshakeNextState, login, login::LoginCompressionS2c, play,
    },
};
use valence_text::IntoText;

use crate::{
    Prev,
    Shutdown,
    // egress::sync_chunks::ChunkSendQueue,
    net::{
        Compose, ConnectionId, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder,
        decoder::BorrowedPacketFrame,
    },
    runtime::AsyncRuntime,
    simulation::{
        packet_state,
        // AiTargetable, ChunkPosition, Comms, ConfirmBlockSequences, EntitySize, IgnMap,
        // ImmuneStatus, Name, Pitch, Player, Position, StreamLookup, Uuid, Velocity, Xp,
        // Yaw,
        // animation::ActiveAnimation,
        // blocks::Blocks,
        // handlers::PacketSwitchQuery,
        // metadata::{MetadataPrefabs, entity::Pose},
        // packet::HandlerRegistry,
        // skin::PlayerSkin,
    },
    // storage::{Events, PlayerJoinServer, SkinHandler},
    util::mojang::MojangClient,
};

/// This event is sent for players who have already been disconnected and about to be destructed. Packets
/// cannot be sent to these players because they have already been disconnected.
///
/// This event cannot be sent to disconnect a player. Use [`crate::net::IoBuf::shutdown`] instead.
#[derive(Event)]
pub struct Disconnect(pub(crate) ());

fn try_next_frame<'a>(
    compose: &'a Compose,
    connection_id: ConnectionId,
    decoder: &'a mut PacketDecoder,
) -> Option<BorrowedPacketFrame<'a>> {
    let bump = compose.bump();
    match decoder.try_next_packet(bump) {
        Ok(Some(packet)) => Some(packet),
        Ok(None) => None,
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

fn try_decode<'a, P: Packet + Decode<'a>>(
    frame: BorrowedPacketFrame<'a>,
    compose: &'a Compose,
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

fn process_handshake(
    mut query: Query<
        '_,
        '_,
        (Entity, &ConnectionId, &mut PacketDecoder),
        With<packet_state::Handshake>,
    >,
    compose: Res<'_, Compose>,
    mut commands: ParallelCommands<'_, '_>,
) {
    query
        .par_iter_mut()
        .for_each(|(entity_id, &connection_id, decoder)| {
            let Some(handshake) = try_next_frame(&*compose, connection_id, decoder.into_inner())
                .map(|frame| {
                    try_decode::<packets::handshaking::HandshakeC2s<'_>>(
                        frame,
                        &*compose,
                        connection_id,
                    )
                })
                .flatten()
            else {
                return;
            };
            commands.command_scope(|mut commands| {
                // todo: check version is correct
                let mut entity = commands.entity(entity_id);
                entity.remove::<packet_state::Handshake>();
                match handshake.next_state {
                    HandshakeNextState::Status => {
                        entity.insert(packet_state::Status(()));
                    }
                    HandshakeNextState::Login => {
                        entity.insert(packet_state::Login(()));
                    }
                }
            });
        });
}
// #[expect(clippy::too_many_arguments, reason = "todo; refactor")]
// fn process_login(
//     world: &WorldRef<'_>,
//     tasks: &AsyncRuntime,
//     login_state: &mut PacketState,
//     decoder: &PacketDecoder,
//     comms: &Comms,
//     skins_collection: SkinHandler,
//     mojang: MojangClient,
//     packet: &BorrowedPacketFrame<'_>,
//     stream_id: ConnectionId,
//     compose: &Compose,
//     entity: &EntityView<'_>,
//     system: EntityView<'_>,
//     ign_map: &IgnMap,
// ) -> anyhow::Result<()> {
//     debug_assert!(
//         *login_state == PacketState::Login,
//         "process_login called with invalid state: {login_state:?}"
//     );
//
//     let login::LoginHelloC2s {
//         username,
//         profile_id,
//     } = packet.decode()?;
//
//     let username = username.0;
//
//     let player_join = PlayerJoinServer {
//         username: username.to_string(),
//         entity: entity.id(),
//     };
//
//     let username = player_join.username.as_str();
//
//     let global = compose.global();
//
//     let pkt = LoginCompressionS2c {
//         threshold: VarInt(global.shared.compression_threshold.0),
//     };
//
//     compose.unicast_no_compression(&pkt, stream_id)?;
//
//     decoder.set_compression(global.shared.compression_threshold);
//
//     let username = Arc::from(username);
//
//     let uuid = profile_id.unwrap_or_else(|| offline_uuid(&username));
//     let uuid_s = format!("{uuid:?}").dimmed();
//     info!("Starting login: {username} {uuid_s}");
//
//     let skins = comms.skins_tx.clone();
//     let entity_id = entity.id();
//
//     if profile_id.is_some() {
//         tasks.spawn(async move {
//             let skin = match PlayerSkin::from_uuid(uuid, &mojang, &skins_collection).await {
//                 Ok(Some(skin)) => skin,
//                 Err(e) => {
//                     error!("failed to get skin {e}. Using empty skin");
//                     PlayerSkin::EMPTY
//                 }
//                 Ok(None) => {
//                     error!("failed to get skin. Using empty skin");
//                     PlayerSkin::EMPTY
//                 }
//             };
//
//             skins.send((entity_id, skin)).unwrap();
//         });
//     } else {
//         skins.send((entity_id, PlayerSkin::EMPTY)).unwrap();
//     }
//
//     let pkt = login::LoginSuccessS2c {
//         uuid,
//         username: Bounded(&username),
//         properties: Cow::default(),
//     };
//
//     compose
//         .unicast(&pkt, stream_id)
//         .context("failed to send login success packet")?;
//
//     *login_state = PacketState::Play;
//
//     ign_map.insert(username.clone(), entity_id, world);
//
//     world.get::<&MetadataPrefabs>(|prefabs| {
//         entity
//             .is_a(prefabs.player_base)
//             .set(Name::from(username))
//             .add(id::<AiTargetable>())
//             .set(ImmuneStatus::default())
//             .set(Uuid::from(uuid))
//             .add(id::<Xp>())
//             .set_pair::<Prev, _>(Xp::default())
//             // .add(id::<ChunkSendQueue>())
//             .add(id::<Velocity>())
//             .set(ChunkPosition::null())
//     });
//
//     compose.io_buf().set_receive_broadcasts(stream_id, world);
//
//     Ok(())
// }
//
// /// Get a [`uuid::Uuid`] based on the given user's name.
// fn offline_uuid(username: &str) -> uuid::Uuid {
//     let digest = sha2::Sha256::digest(username);
//     let digest: [u8; 32] = digest.into();
//     let (&digest, ..) = digest.split_array_ref::<16>();
//
//     // todo: I have no idea which way we should go (be or le)
//     let digest = u128::from_be_bytes(digest);
//     uuid::Uuid::from_u128(digest)
// }
//

fn process_status(
    mut query: Query<'_, '_, (&ConnectionId, &mut PacketDecoder), With<packet_state::Status>>,
    compose: Res<'_, Compose>,
) {
    query
        .par_iter_mut()
        .for_each(|(&connection_id, decoder)| {
            let Some(frame) = try_next_frame(&*compose, connection_id, decoder.into_inner()) else { return };
            match frame.id {
                packets::status::QueryRequestC2s::ID => {
                    let Some(query_request) = try_decode::<packets::status::QueryRequestC2s>(
                        frame,
                        &*compose,
                        connection_id,
                    ) else { return };

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

                    let json = serde_json::to_string_pretty(&json).expect("json serialization should succeed");

                    let send = packets::status::QueryResponseS2c { json: &json };

                    trace!("sent query response: {query_request:?}");
                    compose.unicast_no_compression(&send, connection_id).unwrap();
                }

                packets::status::QueryPingC2s::ID => {
                    let Some(query_ping) = try_decode::<packets::status::QueryPingC2s>(
                        frame,
                        &*compose,
                        connection_id,
                    ) else { return };

                    let payload = query_ping.payload;

                    let send = packets::status::QueryPongS2c { payload };

                    compose.unicast_no_compression(&send, connection_id).unwrap();
                },

                _ => {
                    warn!("player sent invalid packet id during status");
                    compose.io_buf().shutdown(connection_id);
                }
            }
        });
}

pub struct IngressPlugin;

impl Plugin for IngressPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<Disconnect>();
        app.add_systems(Update, (process_handshake, process_status));

        //        world
        //            .system_named::<(&ReceiveState, &ConnectionId, &mut PacketDecoder)>("ingress_to_ecs")
        //            .term_at(0u32)
        //            .singleton() // StreamLookup
        //            .immediate(true)
        //            .kind(id::<flecs::pipeline::PostLoad>())
        //            .each(move |(receive, connection_id, decoder)| {
        //                // 134µs with par_iter
        //                // 150-208µs with regular drain
        //                let span = info_span!("ingress_to_ecs");
        //                let _enter = span.enter();
        //
        //                let Some(mut bytes) = receive.0.packets.get_mut(&connection_id.inner()) else {
        //                    return;
        //                };
        //
        //                if bytes.is_empty() {
        //                    return;
        //                }
        //
        //                decoder.shift_excess();
        //                decoder.queue_slice(bytes.as_ref());
        //                bytes.clear();
        //            });
        //
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
        //        system!(
        //            "recv_data",
        //            world,
        //            &Compose($),
        //            &Blocks($),
        //            &AsyncRuntime($),
        //            &Comms($),
        //            &SkinHandler($),
        //            &MojangClient($),
        //            &HandlerRegistry($),
        //            &mut PacketDecoder,
        //            &mut PacketState,
        //            &ConnectionId,
        //            ?&mut Pose,
        //            &Events($),
        //            &mut EntitySize,
        //            ?&mut Position,
        //            &mut Yaw,
        //            &mut Pitch,
        //            &mut ConfirmBlockSequences,
        //            &mut hyperion_inventory::PlayerInventory,
        //            &mut ActiveAnimation,
        //            &hyperion_crafting::CraftingRegistry($),
        //            &IgnMap($),
        //        )
        //        .kind(id::<flecs::pipeline::OnUpdate>())
        //        .each_iter(
        //            move |it,
        //                  row,
        //                  (
        //                compose,
        //                blocks,
        //                tasks,
        //                comms,
        //                skins_collection,
        //                mojang,
        //                handler_registry,
        //                decoder,
        //                login_state,
        //                &io_ref,
        //                mut pose,
        //                event_queue,
        //                size,
        //                mut position,
        //                yaw,
        //                pitch,
        //                confirm_block_sequences,
        //                inventory,
        //                animation,
        //                crafting_registry,
        //                ign_map,
        //            )| {
        //                let system = it.system();
        //                let world = it.world();
        //                let entity = it.entity(row).expect("row must be in bounds");
        //
        //                let bump = compose.bump.get(&world);
        //
        //                loop {
        //                    let frame = match decoder.try_next_packet(bump) {
        //                        Ok(frame) => frame,
        //                        Err(e) => {
        //                            error!("failed to decode packet: {e}");
        //                            compose.io_buf().shutdown(io_ref, &world);
        //                            break;
        //                        }
        //                    };
        //
        //                    let Some(frame) = frame else {
        //                        break;
        //                    };
        //
        //                    match *login_state {
        //                        PacketState::Handshake => {
        //                            if process_handshake(login_state, &frame).is_err() {
        //                                error!("failed to process handshake");
        //                                compose.io_buf().shutdown(io_ref, &world);
        //                                break;
        //                            }
        //                        }
        //                        PacketState::Status => {
        //                            if let Err(e) =
        //                                process_status(login_state, system, &frame, io_ref, compose)
        //                            {
        //                                error!("failed to process status packet: {e}");
        //                                compose.io_buf().shutdown(io_ref, &world);
        //                                break;
        //                            }
        //                        }
        //                        PacketState::Login => {
        //                            if let Err(e) = process_login(
        //                                &world,
        //                                tasks,
        //                                login_state,
        //                                decoder,
        //                                comms,
        //                                skins_collection.clone(),
        //                                mojang.clone(),
        //                                &frame,
        //                                io_ref,
        //                                compose,
        //                                &entity,
        //                                system,
        //                                ign_map,
        //                            ) {
        //                                error!("failed to process login packet");
        //                                let msg = format!(
        //                                    "§c§lFailed to process login packet:§r\n\n§4{e}§r\n\n§eAre \
        //                                     you on the right version of Minecraft?§r\n§b(Required: \
        //                                     1.20.1)§r"
        //                                );
        //
        //                                // hopefully we were in no compression mode
        //                                // todo we want to handle sending different based on whether
        //                                // we sent compression packet or not
        //                                if let Err(e) = compose.unicast_no_compression(
        //                                    &login::LoginDisconnectS2c {
        //                                        reason: msg.into_cow_text(),
        //                                    },
        //                                    io_ref,
        //                                    system,
        //                                ) {
        //                                    error!("failed to send login disconnect packet: {e}");
        //                                }
        //
        //                                compose.io_buf().shutdown(io_ref, &world);
        //                                break;
        //                            }
        //                        }
        //                        PacketState::Play => {
        //                            // We call this code when you're in play.
        //                            // Transitioning to play is just a way to make sure that the player is officially in play before we start sending them play packets.
        //                            // We have a certain duration that we wait before doing this.
        //                            // todo: better way?
        //                            if let Some((position, pose)) = position.as_mut().zip(pose.as_mut()) {
        //                                let world = &world;
        //                                let id = entity.id();
        //
        //                                let mut query = PacketSwitchQuery {
        //                                    id,
        //                                    view: entity,
        //                                    compose,
        //                                    io_ref,
        //                                    position,
        //                                    yaw,
        //                                    pitch,
        //                                    size,
        //                                    pose,
        //                                    events: event_queue,
        //                                    world,
        //                                    blocks,
        //                                    system,
        //                                    confirm_block_sequences,
        //                                    inventory,
        //                                    animation,
        //                                    crafting_registry,
        //                                    handler_registry,
        //                                };
        //
        //                                // info_span!("ingress", ign = name).in_scope(|| {
        //                                // SAFETY: The packet bytes are allocated in the compose bump
        //                                if let Err(err) = unsafe {
        //                                    crate::simulation::handlers::packet_switch(frame, &mut query)
        //                                } {
        //                                    error!("failed to process packet {frame:?}: {err}");
        //                                }
        //                                // });
        //                            }
        //                        }
        //                        PacketState::Terminate => {
        //                            // todo
        //                        }
        //                    }
        //                }
        //            },
        //        );
    }
}
