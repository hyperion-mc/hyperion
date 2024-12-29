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
        skin::PlayerSkin,
    },
    storage::{Events, GlobalEventHandlers, PlayerJoinServer, SkinHandler},
    util::{SendableRef, TracingExt, mojang::MojangClient},
};

#[derive(Component, Debug)]
pub struct PendingRemove {
    pub reason: String,
}

impl PendingRemove {
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

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
    let id = entity.id();

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

        skins.send((id, skin)).unwrap();
    });

    let pkt = login::LoginSuccessS2c {
        uuid,
        username: Bounded(&username),
        properties: Cow::default(),
    };

    compose
        .unicast(&pkt, stream_id, system)
        .context("failed to send login success packet")?;

    *login_state = PacketState::Play;

    ign_map.insert(username.clone(), entity.id(), world);

    world.get::<&MetadataPrefabs>(|prefabs| {
        entity
            .is_a_id(prefabs.player_base)
            .set(Name::from(username))
            .add::<AiTargetable>()
            .set(ImmuneStatus::default())
            .set(Uuid::from(uuid))
            .add::<Xp>()
            .set_pair::<Prev, _>(Xp::default())
            .add::<ChunkSendQueue>()
            .add::<Velocity>()
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
        .kind::<flecs::pipeline::OnLoad>()
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
        .kind::<flecs::pipeline::OnLoad>()
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
        .kind::<flecs::pipeline::OnLoad>()
        .term_at(0)
        .each_iter(move |it, _, (lookup, receive)| {
            tracing_tracy::client::Client::running()
                .expect("Tracy client should be running")
                .frame_mark();

            let span = info_span!("generate_ingress_events");
            let _enter = span.enter();

            let world = it.world();

            let mut recv = receive.0.lock();

            for connect in recv.player_connect.drain(..) {
                info!("player_connect");
                let view = world
                    .entity()
                    .set(ConnectionId::new(connect))
                    .set(hyperion_inventory::PlayerInventory::default())
                    .set(ConfirmBlockSequences::default())
                    .set(PacketState::Handshake)
                    .set(ActiveAnimation::NONE)
                    .set(PacketDecoder::default())
                    .add::<Player>();

                lookup.insert(connect, view.id());
            }

            for disconnect in recv.player_disconnect.drain(..) {
                // will initiate the removal of entity
                info!("queue pending remove");
                let Some(id) = lookup.get(&disconnect).copied() else {
                    error!("failed to get id for disconnect stream {disconnect:?}");
                    continue;
                };
                world
                    .entity_from_id(*id)
                    .set(PendingRemove::new("disconnected"));
            }
        });

        #[expect(
            clippy::unwrap_used,
            reason = "this is only called once on startup; it should be fine. we mostly care \
                      about crashing during server execution"
        )]
        let num_threads = i32::try_from(rayon::current_num_threads()).unwrap();

        let worlds = (0..num_threads)
            // SAFETY: promoting world to static lifetime, system won't outlive world
            .map(|i| unsafe { std::mem::transmute(world.stage(i)) })
            .map(SendableRef)
            .collect::<Vec<_>>();

        system!(
            "ingress_to_ecs",
            world,
            &StreamLookup($),
            &ReceiveState($),
        )
        .immediate(true)
        .kind::<flecs::pipeline::PostLoad>()
        .each(move |(lookup, receive)| {
            use rayon::prelude::*;

            // 134µs with par_iter
            // 150-208µs with regular drain
            let span = info_span!("ingress_to_ecs");
            let _enter = span.enter();

            let mut recv = receive.0.lock();

            recv.packets.par_drain().for_each(|(entity_id, bytes)| {
                #[expect(
                    clippy::indexing_slicing,
                    reason = "it should be impossible to get a thread index that is out of bounds \
                              unless the rayon thread pool changes size which does not occur"
                )]
                let world = &worlds[rayon::current_thread_index().unwrap_or_default()];
                let world = &world.0;

                let Some(entity_id) = lookup.get(&entity_id) else {
                    // this is not necessarily a bug; race conditions occur
                    warn!("player_packets: entity for {entity_id:?}");
                    return;
                };

                if !world.is_alive(*entity_id) {
                    return;
                }

                let entity = world.entity_from_id(*entity_id);

                entity.get::<&mut PacketDecoder>(|decoder| {
                    decoder.shift_excess();
                    decoder.queue_slice(bytes.as_ref());
                });
            });
        });

        system!(
            "remove_player_from_visibility",
            world,
            &Uuid,
            &Compose($),
            &ConnectionId,
            &PendingRemove,
        )
        .kind::<flecs::pipeline::PostLoad>()
        .each_iter(move |it, row, (uuid, compose, io, pending_remove)| {
            let system = it.system();
            let entity = it.entity(row);
            let uuids = &[uuid.0];
            let entity_ids = [VarInt(entity.minecraft_id())];

            // destroy
            let pkt = play::EntitiesDestroyS2c {
                entity_ids: Cow::Borrowed(&entity_ids),
            };

            if let Err(e) = compose.broadcast(&pkt, system).send() {
                error!("failed to send player remove packet: {e}");
                return;
            };

            let pkt = play::PlayerRemoveS2c {
                uuids: Cow::Borrowed(uuids),
            };

            if let Err(e) = compose.broadcast(&pkt, system).send() {
                error!("failed to send player remove packet: {e}");
            };

            if !pending_remove.reason.is_empty() {
                let pkt = play::DisconnectS2c {
                    reason: pending_remove.reason.clone().into_cow_text(),
                };

                if let Err(e) = compose.unicast_no_compression(&pkt, *io, system) {
                    error!("failed to send disconnect packet: {e}");
                }
            }
        });

        world
            .system_named::<()>("remove_player")
            .kind::<flecs::pipeline::PostLoad>()
            .with::<&PendingRemove>()
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
            &GlobalEventHandlers($),
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
        .kind::<flecs::pipeline::OnUpdate>()
        .multi_threaded()
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
                handlers,
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
                let entity = it.entity(row);

                let bump = compose.bump.get(&world);

                loop {
                    let frame = match decoder.try_next_packet(bump) {
                        Ok(frame) => frame,
                        Err(e) => {
                            error!("failed to decode packet: {e}");
                            entity.destruct();
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

                                entity.destruct();

                                break;
                            }
                        }
                        PacketState::Status => {
                            if let Err(e) =
                                process_status(login_state, system, &frame, io_ref, compose)
                            {
                                error!("failed to process status packet: {e}");
                                entity.destruct();
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

                                entity.destruct();
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

                                let mut query = PacketSwitchQuery {
                                    id: entity.id(),
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
                                    handlers,
                                };

                                // info_span!("ingress", ign = name).in_scope(|| {
                                if let Err(err) =
                                    crate::simulation::handlers::packet_switch(frame, &mut query)
                                {
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
