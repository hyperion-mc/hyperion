//! The protocol-776 pre-play state machine: handshake, status, login and
//! configuration.
//!
//! One system per state, each single-threaded. The 763 path splits decoding
//! (parallel, `PreUpdate`) from acting on the packet (`OnUpdate`) because its
//! generated queues let it; these systems spawn entities and change protocol
//! state, which flecs forbids from a multithreaded system, so they decode and
//! act in one pass. Pre-play traffic is a handful of packets per connection,
//! not per tick.
//!
//! The states are driven entirely by what the client sends, so each system
//! reads at most one packet per tick in the states that can transition. Reading
//! further would decode the next packet against the state it was sent in, not
//! the state it was received in.

use std::sync::Arc;

use colored::Colorize;
use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::{
        self, configuration::serverbound::PacketId as ConfigPacketId,
        login::serverbound::PacketId as LoginPacketId,
        status::serverbound::PacketId as StatusPacketId,
    },
    packets::{
        common::serverbound::PingRequest,
        configuration::{
            self, ClientInformation, UpdateTags,
            clientbound::{FinishConfiguration, RegistryData, UpdateEnabledFeatures},
            serverbound::SelectKnownPacks,
        },
        handshake::serverbound::Intention,
        login::{
            clientbound::{LoginCompression, LoginFinished},
            serverbound::Hello,
        },
        status::clientbound::{PongResponse, StatusResponse},
    },
    types::{
        ClientIntent, GameProfile, Identifier, Uuid as ProtoUuid,
        registry_synchronization::PackedRegistryEntry,
    },
};
use serde_json::json;
use tracing::{error, info, warn};

use crate::{
    Prev,
    egress::sync_chunks::ChunkSendQueue,
    ingress::{ServerPingResponse, decode::Decompressor},
    net::{
        Compose, ConnectionId, MINECRAFT_VERSION, PROTOCOL_VERSION, PacketDecoder,
        decoder::BorrowedPacketFrame,
        protocol::{
            decode_body, frame_body, join, known_packs, registries, send, send_uncompressed,
        },
    },
    runtime::AsyncRuntime,
    simulation::{
        AiTargetable, ChunkPosition, Comms, ConfirmBlockSequences, IgnMap, ImmuneStatus, Name,
        PacketState, Player, Uuid, Velocity, Xp, animation::ActiveAnimation,
        entity_kind::EntityKind, metadata::MetadataPrefabs, skin::PlayerSkin,
    },
    storage::SkinHandler,
    util::mojang::MojangClient,
};

/// Whether a client reported the data packs this server would have sent
/// contents for.
///
/// Set from the serverbound `select_known_packs` and read when building
/// `registry_data`: see [`registries`] for why the answer decides whether this
/// server can serve the client at all.
#[derive(Component, Debug, Default)]
pub struct KnownPacksAccepted(pub bool);

/// Read the next frame for a connection, shutting it down on a decode error.
fn next_frame(
    compose: &Compose,
    connection_id: ConnectionId,
    decoder: &PacketDecoder,
    decompressor: &mut libdeflater::Decompressor,
    receiver: &mut packet_channel::Receiver,
) -> Option<BorrowedPacketFrame> {
    let raw = receiver.try_recv()?;
    match decoder.try_next_packet(decompressor, raw) {
        Ok(frame) => Some(frame),
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

fn handshake(world: &World) {
    world
        .system_named::<(
            &Compose,
            &Decompressor,
            &ConnectionId,
            &PacketDecoder,
            &mut packet_channel::Receiver,
        )>("handshake")
        .with_enum(PacketState::Handshake)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_entity(
            |entity, (compose, decompressor, &connection_id, decoder, receiver)| {
                let mut decompressor = decompressor.0.get_or_default().borrow_mut();
                let Some(frame) =
                    next_frame(compose, connection_id, decoder, &mut decompressor, receiver)
                else {
                    return;
                };

                let expected = packet_id::handshake::serverbound::PacketId::Intention.to_raw();
                if frame.id != expected {
                    error!(
                        "handshake: expected intention (id {expected}), got {}",
                        frame.id
                    );
                    compose.io_buf().shutdown(connection_id);
                    return;
                }

                let intention = match decode_body::<Intention<'_>>(frame_body(&frame)) {
                    Ok(intention) => intention,
                    Err(e) => {
                        error!("handshake: failed to decode intention: {e}");
                        compose.io_buf().shutdown(connection_id);
                        return;
                    }
                };

                // A mismatched version is only worth refusing on the login
                // path. A client on any version may ping, and answering the
                // ping is how it learns which version to be.
                if intention.intention != ClientIntent::Status
                    && intention.protocol_version != PROTOCOL_VERSION
                {
                    warn!(
                        "client speaks protocol {} but this server speaks {PROTOCOL_VERSION}",
                        intention.protocol_version
                    );
                }

                // PacketState is an exclusive relationship, so adding the next
                // state removes the handshake state.
                let next = match intention.intention {
                    ClientIntent::Status => PacketState::Status,
                    // A transfer arrives already authenticated elsewhere, but
                    // it still sends `hello` next, so it enters login the same
                    // way a fresh connection does.
                    ClientIntent::Login | ClientIntent::Transfer => PacketState::Login,
                };
                entity.add_enum(next);
            },
        );
}

fn status(world: &World) {
    world
        .system_named::<(
            &Compose,
            &ServerPingResponse,
            &Decompressor,
            &ConnectionId,
            &PacketDecoder,
            &mut packet_channel::Receiver,
        )>("status")
        .with_enum(PacketState::Status)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(
            |(compose, ping_response, decompressor, &connection_id, decoder, receiver)| {
                let mut decompressor = decompressor.0.get_or_default().borrow_mut();

                // Status cannot transition, so every queued packet is safe to
                // read in one pass.
                while let Some(frame) =
                    next_frame(compose, connection_id, decoder, &mut decompressor, receiver)
                {
                    let result = match StatusPacketId::from_raw(frame.id) {
                        Some(StatusPacketId::StatusRequest) => {
                            status_response(compose, connection_id, ping_response)
                        }
                        Some(StatusPacketId::PingRequest) => {
                            status_pong(compose, connection_id, frame_body(&frame))
                        }
                        other => Err(anyhow::anyhow!("unexpected status packet {other:?}")),
                    };

                    if let Err(e) = result {
                        error!("status: {e}");
                        compose.io_buf().shutdown(connection_id);
                        return;
                    }
                }
            },
        );
}

fn status_response(
    compose: &Compose,
    connection_id: ConnectionId,
    ping_response: &ServerPingResponse,
) -> anyhow::Result<()> {
    let online = compose
        .global()
        .player_count
        .load(std::sync::atomic::Ordering::Relaxed);

    // `description` is a chat component since 1.20.3, so it goes as an object
    // rather than a bare string; a client shows a blank MOTD for the latter.
    let json = json!({
        "version": { "name": MINECRAFT_VERSION, "protocol": PROTOCOL_VERSION },
        "players": { "online": online, "max": ping_response.max_players, "sample": [] },
        "description": { "text": ping_response.description },
    });
    let json = serde_json::to_string(&json)?;

    send_uncompressed(
        compose,
        connection_id,
        packet_id::status::clientbound::PacketId::StatusResponse.to_raw(),
        &StatusResponse { status: &json },
    )
}

fn status_pong(compose: &Compose, connection_id: ConnectionId, body: &[u8]) -> anyhow::Result<()> {
    let ping = decode_body::<PingRequest>(body)?;
    send_uncompressed(
        compose,
        connection_id,
        packet_id::status::clientbound::PacketId::PongResponse.to_raw(),
        &PongResponse(ping.0),
    )
}

fn login(world: &World) {
    world
        .system_named::<(
            &Compose,
            &AsyncRuntime,
            &SkinHandler,
            &MojangClient,
            &IgnMap,
            &Comms,
            &Decompressor,
            &ConnectionId,
            &mut PacketDecoder,
            &mut packet_channel::Receiver,
        )>("login")
        .with_enum(PacketState::Login)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(
            |it,
             row,
             (
                compose,
                runtime,
                skins,
                mojang,
                ign_map,
                comms,
                decompressor,
                &connection_id,
                decoder,
                receiver,
            )| {
                let world = it.world();
                let entity = it.entity(row);
                let mut decompressor = decompressor.0.get_or_default().borrow_mut();

                let Some(frame) =
                    next_frame(compose, connection_id, decoder, &mut decompressor, receiver)
                else {
                    return;
                };

                let result = match LoginPacketId::from_raw(frame.id) {
                    Some(LoginPacketId::Hello) => login_hello(
                        &world,
                        entity,
                        compose,
                        runtime,
                        skins,
                        mojang,
                        ign_map,
                        comms,
                        decoder,
                        connection_id,
                        frame_body(&frame),
                    ),
                    Some(LoginPacketId::LoginAcknowledged) => {
                        entity.add_enum(PacketState::Configuration);
                        start_configuration(compose, connection_id)
                    }
                    other => Err(anyhow::anyhow!("unexpected login packet {other:?}")),
                };

                if let Err(e) = result {
                    error!("login: {e}");
                    compose.io_buf().shutdown(connection_id);
                }
            },
        );
}

/// Build the profile id for an offline-mode player.
///
/// Kept identical to the 763 path so a player keeps the same id across the two
/// protocols, which is what makes anything keyed on it -- permissions, stats --
/// survive the switch.
fn offline_uuid(username: &str) -> uuid::Uuid {
    crate::ingress::offline_uuid(username)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one flecs system's worth of singletons"
)]
fn login_hello(
    world: &WorldRef<'_>,
    entity: EntityView<'_>,
    compose: &Compose,
    runtime: &AsyncRuntime,
    skins: &SkinHandler,
    mojang: &MojangClient,
    ign_map: &IgnMap,
    comms: &Comms,
    decoder: &mut PacketDecoder,
    connection_id: ConnectionId,
    body: &[u8],
) -> anyhow::Result<()> {
    let hello = decode_body::<Hello<'_>>(body)?;
    let username: Arc<str> = Arc::from(hello.name);

    // Compression is negotiated before anything else, because the client
    // applies it to the very next frame it reads.
    let threshold = compose.global().shared.compression_threshold;
    send_uncompressed(
        compose,
        connection_id,
        packet_id::login::clientbound::PacketId::LoginCompression.to_raw(),
        &LoginCompression(threshold.0),
    )?;
    decoder.set_compression(threshold);

    // `Hello.profile_id` is what the launcher cached, not proof of anything:
    // this server is offline-mode, so a zero id means derive one from the name.
    let uuid = if hello.profile_id.0 == 0 {
        offline_uuid(&username)
    } else {
        uuid::Uuid::from_u128(hello.profile_id.0)
    };

    info!(
        "Starting login: {:?} {username} {}",
        entity.id(),
        format!("{uuid:?}").dimmed()
    );

    send(
        compose,
        connection_id,
        packet_id::login::clientbound::PacketId::LoginFinished.to_raw(),
        &LoginFinished {
            game_profile: GameProfile {
                id: ProtoUuid(uuid.as_u128()),
                name: &username,
                properties: Vec::new(),
            },
            // The chat session is not used yet; a zero id is what a server
            // that does not sign chat sends.
            session_id: ProtoUuid(0),
        },
    )?;

    let skin = if hello.profile_id.0 == 0 {
        Some(PlayerSkin::EMPTY)
    } else {
        let mojang = mojang.clone();
        let skins = skins.clone();
        let skins_tx = comms.skins_tx.clone();
        let sender = entity.id();

        runtime.spawn(async move {
            let skin = match PlayerSkin::from_uuid(uuid, &mojang, &skins).await {
                Ok(Some(skin)) => skin,
                Ok(None) => {
                    error!("failed to get skin. Using empty skin");
                    PlayerSkin::EMPTY
                }
                Err(e) => {
                    error!("failed to get skin {e}. Using empty skin");
                    PlayerSkin::EMPTY
                }
            };

            if let Err(e) = skins_tx.send((sender, skin)) {
                error!("failed to hand skin to the join system: {e}");
            }
        });

        None
    };

    ign_map.insert(username.clone(), entity.id(), world);

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
            .set(KnownPacksAccepted::default())
            .add_enum(EntityKind::Player)
            .add(id::<Player>());
    });

    if let Some(skin) = skin
        && let Err(e) = comms.skins_tx.send((entity.id(), skin))
    {
        error!("failed to hand skin to the join system: {e}");
    }

    Ok(())
}

/// The three packets `ServerConfigurationPacketListenerImpl.startConfiguration`
/// sends before it waits on the client.
fn start_configuration(compose: &Compose, connection_id: ConnectionId) -> anyhow::Result<()> {
    use packet_id::configuration::clientbound::PacketId;

    // `BrandPayload`: a single string, under `minecraft:brand`.
    let mut brand = hyperion_minecraft_proto::Writer::new();
    brand.string("hyperion")?;
    let brand = brand.into_vec();
    send(
        compose,
        connection_id,
        PacketId::CustomPayload.to_raw(),
        &configuration::CustomPayload {
            channel: "minecraft:brand",
            data: &brand,
        },
    )?;

    send(
        compose,
        connection_id,
        PacketId::UpdateEnabledFeatures.to_raw(),
        &UpdateEnabledFeatures(vec![Identifier::new("minecraft:vanilla")?]),
    )?;

    send(
        compose,
        connection_id,
        PacketId::SelectKnownPacks.to_raw(),
        &configuration::clientbound::SelectKnownPacks {
            known_packs: known_packs(),
        },
    )
}

fn configuration(world: &World) {
    world
        .system_named::<(
            &Compose,
            &Decompressor,
            &ConnectionId,
            &PacketDecoder,
            &mut packet_channel::Receiver,
            &mut KnownPacksAccepted,
        )>("configuration")
        .with_enum(PacketState::Configuration)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(|it, row, (
            compose,
            decompressor,
            &connection_id,
            decoder,
            receiver,
            known_packs_accepted,
        )| {
            let world = it.world();
            let entity = it.entity(row);
            let mut decompressor = decompressor.0.get_or_default().borrow_mut();

            let Some(frame) =
                next_frame(compose, connection_id, decoder, &mut decompressor, receiver)
            else {
                return;
            };

            let result = match ConfigPacketId::from_raw(frame.id) {
                Some(ConfigPacketId::SelectKnownPacks) => select_known_packs(
                    compose,
                    connection_id,
                    known_packs_accepted,
                    frame_body(&frame),
                ),
                Some(ConfigPacketId::ClientInformation) => {
                    client_information(entity, frame_body(&frame))
                }
                Some(ConfigPacketId::FinishConfiguration) => {
                    entity.add_enum(PacketState::Play);
                    join::enter_world(&world, entity, compose, connection_id)
                }
                // A client answers a keep-alive and may send a payload on its
                // own channel; neither changes the handover, so both are read
                // and dropped rather than treated as a protocol error.
                Some(
                    ConfigPacketId::KeepAlive
                    | ConfigPacketId::Pong
                    | ConfigPacketId::CustomPayload,
                ) => Ok(()),
                other => Err(anyhow::anyhow!("unexpected configuration packet {other:?}")),
            };

            if let Err(e) = result {
                error!("configuration: {e}");
                compose.io_buf().shutdown(connection_id);
            }
        });
}

fn select_known_packs(
    compose: &Compose,
    connection_id: ConnectionId,
    accepted: &mut KnownPacksAccepted,
    body: &[u8],
) -> anyhow::Result<()> {
    use packet_id::configuration::clientbound::PacketId;

    let response = decode_body::<SelectKnownPacks<'_>>(body)?;
    accepted.0 = response.known_packs == known_packs();

    // `SynchronizeRegistriesTask.handleResponse` sends full contents when the
    // client did not accept every offered pack. This server has no contents to
    // send -- see the module docs on `registries` -- so it says so plainly
    // rather than sending entries the client cannot resolve and letting it
    // fail somewhere less obvious.
    anyhow::ensure!(
        accepted.0,
        "client did not report the vanilla core data pack ({:?}); serving it would need registry \
         contents this server does not have yet",
        response.known_packs
    );

    for registry in registries::SYNCHRONIZED {
        let entries = registry
            .entries
            .iter()
            .map(|&id| PackedRegistryEntry { id, data: None })
            .collect();

        send(
            compose,
            connection_id,
            PacketId::RegistryData.to_raw(),
            &RegistryData {
                registry: registry.name,
                entries,
            },
        )?;
    }

    // Vanilla follows the registries with the tag sets. Sending none is
    // well-formed: the client keeps the tags it loaded from its own packs.
    send(
        compose,
        connection_id,
        PacketId::UpdateTags.to_raw(),
        &UpdateTags { tags: Vec::new() },
    )?;

    send(
        compose,
        connection_id,
        PacketId::FinishConfiguration.to_raw(),
        &FinishConfiguration,
    )
}

/// Apply the client's own render settings.
///
/// The skin overlay mask lives here and nowhere else. A server that leaves this
/// packet unhandled leaves `DisplayedSkinParts` at zero, and every player
/// renders with no hat, jacket or sleeves. 1.20.2 moved the packet into
/// configuration, so it arrives before the player is in the world.
fn client_information(entity: EntityView<'_>, body: &[u8]) -> anyhow::Result<()> {
    use crate::simulation::metadata::player::{DisplayedSkinParts, MainHand};

    let info = decode_body::<ClientInformation<'_>>(body)?;

    entity
        .set(DisplayedSkinParts::new(info.model_customisation))
        .set(MainHand::new(info.main_hand.to_raw().to_le_bytes()[0]));

    Ok(())
}

/// Registers the pre-play systems.
#[derive(Component)]
pub struct PrePlayModule;

impl Module for PrePlayModule {
    fn module(world: &World) {
        world.component::<KnownPacksAccepted>();

        handshake(world);
        status(world);
        login(world);
        configuration(world);
    }
}
