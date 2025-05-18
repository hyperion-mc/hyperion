use std::{borrow::Cow, sync::Arc};

use anyhow::Context;
use colored::Colorize;
use flecs_ecs::prelude::*;
use hyperion_utils::EntityExt;
use serde_json::json;
use sha2::Digest;
use tracing::{error, info, info_span, trace, warn};
use valence_protocol::{
    Bounded, Packet, VarInt, packets,
    packets::{
        handshaking::handshake_c2s::HandshakeNextState, login, login::LoginCompressionS2c, play,
    },
};
use valence_text::IntoText;

use crate::{
    Prev, Shutdown,
    egress::sync_chunks::ChunkSendQueue,
    net::{
        Compose, ConnectionId, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder,
        decoder::BorrowedPacketFrame, proxy::ReceiveState,
    },
    runtime::AsyncRuntime,
    simulation::{
        AiTargetable, ChunkPosition, Comms, ConfirmBlockSequences, EntitySize, IgnMap,
        ImmuneStatus, Name, PacketState, Pitch, Player, Position, StreamLookup, Uuid, Velocity, Xp,
        Yaw,
        animation::ActiveAnimation,
        blocks::Blocks,
        handlers::PacketSwitchQuery,
        metadata::{MetadataPrefabs, entity::Pose},
        packet::HandlerRegistry,
        skin::PlayerSkin,
    },
    storage::{Events, PlayerJoinServer, SkinHandler},
    util::{TracingExt, mojang::MojangClient},
};

/// This marks players who have already been disconnected and about to be destructed. This component should not be
/// added to an entity to disconnect a player. Use [`crate::net::IoBuf::shutdown`] instead.
#[derive(Component, Debug)]
pub struct PendingRemove;

fn process_handshake(
    login_state: &mut PacketState,
    packet: &BorrowedPacketFrame<'_>,
) -> anyhow::Result<()> {
    debug_assert!(
        *login_state == PacketState::Handshake,
        "process_handshake called with invalid state: {login_state:?}"
    );

    let handshake: packets::handshaking::HandshakeC2s<'_> = packet.decode()?;

    // todo: check version is correct
    match handshake.next_state {
        HandshakeNextState::Status => {
            *login_state = PacketState::Status;
        }
        HandshakeNextState::Login => {
            *login_state = PacketState::Login;
        }
    }

    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "todo; refactor")]
fn process_login(
    world: &WorldRef<'_>,
    tasks: &AsyncRuntime,
    login_state: &mut PacketState,
    decoder: &PacketDecoder,
    comms: &Comms,
    skins_collection: SkinHandler,
    mojang: MojangClient,
    packet: &BorrowedPacketFrame<'_>,
    stream_id: ConnectionId,
    compose: &Compose,
    entity: &EntityView<'_>,
    system: EntityView<'_>,
    ign_map: &IgnMap,
) -> anyhow::Result<()> {
    debug_assert!(
        *login_state == PacketState::Login,
        "process_login called with invalid state: {login_state:?}"
    );

    let login::LoginHelloC2s {
        username,
        profile_id,
    } = packet.decode()?;

    let username = username.0;

    let player_join = PlayerJoinServer {
        username: username.to_string(),
        entity: entity.id(),
    };

    let username = player_join.username.as_str();

    let global = compose.global();

    let pkt = LoginCompressionS2c {
        threshold: VarInt(global.shared.compression_threshold.0),
    };

    compose.unicast_no_compression(&pkt, stream_id, system)?;

    decoder.set_compression(global.shared.compression_threshold);

    let username = Arc::from(username);

    let uuid = profile_id.unwrap_or_else(|| offline_uuid(&username));
    let uuid_s = format!("{uuid:?}").dimmed();
    info!("Starting login: {username} {uuid_s}");

    let skins = comms.skins_tx.clone();
    let entity_id = entity.id();

    if profile_id.is_some() {
        tasks.spawn(async move {
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

            skins.send((entity_id, skin)).unwrap();
        });
    } else {
        skins.send((entity_id, PlayerSkin::EMPTY)).unwrap();
    }

    let pkt = login::LoginSuccessS2c {
        uuid,
        username: Bounded(&username),
        properties: Cow::default(),
    };

    compose
        .unicast(&pkt, stream_id, system)
        .context("failed to send login success packet")?;

    *login_state = PacketState::Play;

    ign_map.insert(username.clone(), entity_id, world);

    world.get::<&MetadataPrefabs>(|prefabs| {
        entity
            .is_a(prefabs.player_base)
            .set(Name::from(username))
            .add(id::<AiTargetable>())
            .set(ImmuneStatus::default())
            .set(Uuid::from(uuid))
            .add(id::<Xp>())
            .set_pair::<Prev, _>(Xp::default())
            .add(id::<ChunkSendQueue>())
            .add(id::<Velocity>())
            .set(ChunkPosition::null())
    });

    compose.io_buf().set_receive_broadcasts(stream_id, world);

    Ok(())
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

fn process_status(
    login_state: &mut PacketState,
    system: EntityView<'_>,
    packet: &BorrowedPacketFrame<'_>,
    packets: ConnectionId,
    compose: &Compose,
) -> anyhow::Result<()> {
    debug_assert!(
        *login_state == PacketState::Status,
        "process_status called with invalid state: {login_state:?}"
    );

    match packet.id {
        packets::status::QueryRequestC2s::ID => {
            let query_request: packets::status::QueryRequestC2s = packet.decode()?;

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

            let json = serde_json::to_string_pretty(&json)?;

            let send = packets::status::QueryResponseS2c { json: &json };

            trace!("sent query response: {query_request:?}");
            compose.unicast_no_compression(&send, packets, system)?;
        }

        packets::status::QueryPingC2s::ID => {
            let query_ping: packets::status::QueryPingC2s = packet.decode()?;

            let payload = query_ping.payload;

            let send = packets::status::QueryPongS2c { payload };

            compose.unicast_no_compression(&send, packets, system)?;

            trace!("sent query pong: {query_ping:?}");
            *login_state = PacketState::Terminate;
        }

        _ => warn!("unexpected packet id during status: {packet:?}"),
    }

    // todo: check version is correct

    Ok(())
}

#[derive(Component)]
pub struct IngressModule;

impl Module for IngressModule {
    #[expect(clippy::too_many_lines)]
    fn module(world: &World) {
        system!(
            "shutdown",
            world,
            &Shutdown($),
        )
        .kind(id::<flecs::pipeline::OnLoad>())
        .each_iter(|it, _, shutdown| {
            let world = it.world();
            if shutdown.value.load(std::sync::atomic::Ordering::Relaxed) {
                info!("shutting down");
                world.quit();
            }
        });

        system!(
            "update_ign_map",
            world,
            &mut IgnMap($),
        )
        .kind(id::<flecs::pipeline::OnLoad>())
        .each_iter(|_, _, ign_map| {
            let span = info_span!("update_ign_map");
            let _enter = span.enter();
            ign_map.update();
        });

        system!(
            "generate_ingress_events",
            world,
            &mut StreamLookup($),
            &ReceiveState($),
        )
        .immediate(true)
        .kind(id::<flecs::pipeline::OnLoad>())
        .term_at(0)
        .each_iter(move |it, _, (lookup, receive)| {
            tracing_tracy::client::Client::running()
                .expect("Tracy client should be running")
                .frame_mark();

            let span = info_span!("generate_ingress_events");
            let _enter = span.enter();

            let world = it.world();

            let recv = &receive.0;

            for connect in recv.player_connect.lock().drain(..) {
                info!("player_connect");
                let view = world
                    .entity()
                    .set(ConnectionId::new(connect))
                    .set(hyperion_inventory::PlayerInventory::default())
                    .set(ConfirmBlockSequences::default())
                    .set(PacketState::Handshake)
                    .set(ActiveAnimation::NONE)
                    .set(PacketDecoder::default())
                    .add(id::<Player>());

                lookup.insert(connect, view.id());
            }

            for disconnect in recv.player_disconnect.lock().drain(..) {
                // will initiate the removal of entity
                info!("queue pending remove");
                let Some(entity_id) = lookup.get(&disconnect).copied() else {
                    error!("failed to get id for disconnect stream {disconnect:?}");
                    continue;
                };
                world.entity_from_id(*entity_id).add(id::<PendingRemove>());
            }
        });

        world
            .system_named::<(&ReceiveState, &ConnectionId, &mut PacketDecoder)>("ingress_to_ecs")
            .term_at(0u32)
            .singleton() // StreamLookup
            .immediate(true)
            .kind(id::<flecs::pipeline::PostLoad>())
            .each(move |(receive, connection_id, decoder)| {
                // 134µs with par_iter
                // 150-208µs with regular drain
                let span = info_span!("ingress_to_ecs");
                let _enter = span.enter();

                let Some(mut bytes) = receive.0.packets.get_mut(&connection_id.inner()) else {
                    return;
                };

                if bytes.is_empty() {
                    return;
                }

                decoder.shift_excess();
                decoder.queue_slice(bytes.as_ref());
                bytes.clear();
            });

        system!(
            "remove_player_from_visibility",
            world,
            &Uuid,
            &Compose($),
        )
        .kind(id::<flecs::pipeline::PostLoad>())
        .with(id::<PendingRemove>())
        .each_iter(move |it, row, (uuid, compose)| {
            let system = it.system();
            let entity = it.entity(row).expect("row must be in bounds");
            let uuids = &[uuid.0];
            let entity_ids = [VarInt(entity.minecraft_id())];

            // destroy
            let pkt = play::EntitiesDestroyS2c {
                entity_ids: Cow::Borrowed(&entity_ids),
            };

            if let Err(e) = compose.broadcast(&pkt, system).send() {
                error!("failed to send player remove packet: {e}");
                return;
            }

            let pkt = play::PlayerRemoveS2c {
                uuids: Cow::Borrowed(uuids),
            };

            if let Err(e) = compose.broadcast(&pkt, system).send() {
                error!("failed to send player remove packet: {e}");
            }
        });

        world
            .system_named::<()>("remove_player")
            .kind(id::<flecs::pipeline::PostLoad>())
            .with(id::<&PendingRemove>())
            .tracing_each_entity(info_span!("remove_player"), |entity, ()| {
                entity.destruct();
            });

        system!(
            "recv_data",
            world,
            &Compose($),
            &Blocks($),
            &AsyncRuntime($),
            &Comms($),
            &SkinHandler($),
            &MojangClient($),
            &HandlerRegistry($),
            &mut PacketDecoder,
            &mut PacketState,
            &ConnectionId,
            ?&mut Pose,
            &Events($),
            &mut EntitySize,
            ?&mut Position,
            &mut Yaw,
            &mut Pitch,
            &mut ConfirmBlockSequences,
            &mut hyperion_inventory::PlayerInventory,
            &mut ActiveAnimation,
            &hyperion_crafting::CraftingRegistry($),
            &IgnMap($),
        )
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(
            move |it,
                  row,
                  (
                compose,
                blocks,
                tasks,
                comms,
                skins_collection,
                mojang,
                handler_registry,
                decoder,
                login_state,
                &io_ref,
                mut pose,
                event_queue,
                size,
                mut position,
                yaw,
                pitch,
                confirm_block_sequences,
                inventory,
                animation,
                crafting_registry,
                ign_map,
            )| {
                let system = it.system();
                let world = it.world();
                let entity = it.entity(row).expect("row must be in bounds");

                let bump = compose.bump.get(&world);

                loop {
                    let frame = match decoder.try_next_packet(bump) {
                        Ok(frame) => frame,
                        Err(e) => {
                            error!("failed to decode packet: {e}");
                            compose.io_buf().shutdown(io_ref, &world);
                            break;
                        }
                    };

                    let Some(frame) = frame else {
                        break;
                    };

                    match *login_state {
                        PacketState::Handshake => {
                            if process_handshake(login_state, &frame).is_err() {
                                error!("failed to process handshake");
                                compose.io_buf().shutdown(io_ref, &world);
                                break;
                            }
                        }
                        PacketState::Status => {
                            if let Err(e) =
                                process_status(login_state, system, &frame, io_ref, compose)
                            {
                                error!("failed to process status packet: {e}");
                                compose.io_buf().shutdown(io_ref, &world);
                                break;
                            }
                        }
                        PacketState::Login => {
                            if let Err(e) = process_login(
                                &world,
                                tasks,
                                login_state,
                                decoder,
                                comms,
                                skins_collection.clone(),
                                mojang.clone(),
                                &frame,
                                io_ref,
                                compose,
                                &entity,
                                system,
                                ign_map,
                            ) {
                                error!("failed to process login packet");
                                let msg = format!(
                                    "§c§lFailed to process login packet:§r\n\n§4{e}§r\n\n§eAre \
                                     you on the right version of Minecraft?§r\n§b(Required: \
                                     1.20.1)§r"
                                );

                                // hopefully we were in no compression mode
                                // todo we want to handle sending different based on whether
                                // we sent compression packet or not
                                if let Err(e) = compose.unicast_no_compression(
                                    &login::LoginDisconnectS2c {
                                        reason: msg.into_cow_text(),
                                    },
                                    io_ref,
                                    system,
                                ) {
                                    error!("failed to send login disconnect packet: {e}");
                                }

                                compose.io_buf().shutdown(io_ref, &world);
                                break;
                            }
                        }
                        PacketState::Play => {
                            // We call this code when you're in play.
                            // Transitioning to play is just a way to make sure that the player is officially in play before we start sending them play packets.
                            // We have a certain duration that we wait before doing this.
                            // todo: better way?
                            if let Some((position, pose)) = position.as_mut().zip(pose.as_mut()) {
                                let world = &world;
                                let id = entity.id();

                                let mut query = PacketSwitchQuery {
                                    id,
                                    view: entity,
                                    compose,
                                    io_ref,
                                    position,
                                    yaw,
                                    pitch,
                                    size,
                                    pose,
                                    events: event_queue,
                                    world,
                                    blocks,
                                    system,
                                    confirm_block_sequences,
                                    inventory,
                                    animation,
                                    crafting_registry,
                                    handler_registry,
                                };

                                // info_span!("ingress", ign = name).in_scope(|| {
                                // SAFETY: The packet bytes are allocated in the compose bump
                                if let Err(err) = unsafe {
                                    crate::simulation::handlers::packet_switch(frame, &mut query)
                                } {
                                    error!("failed to process packet {frame:?}: {err}");
                                }
                                // });
                            }
                        }
                        PacketState::Terminate => {
                            // todo
                        }
                    }
                }
            },
        );
    }
}
