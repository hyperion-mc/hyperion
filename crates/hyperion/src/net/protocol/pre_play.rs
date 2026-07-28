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
            self, ClientInformation,
            clientbound::{FinishConfiguration, RegistryData, UpdateEnabledFeatures},
            serverbound::SelectKnownPacks,
        },
        handshake::serverbound::Intention,
        login::{
            clientbound::{LoginCompression, LoginDisconnect, LoginFinished},
            serverbound::Hello,
        },
        status::clientbound::{PongResponse, StatusResponse},
    },
    tag_data,
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
        entity_kind::EntityKind, gamemode::DefaultGamemode, metadata::MetadataPrefabs,
        skin::PlayerSkin,
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
                        "refusing client on protocol {}; this server speaks {PROTOCOL_VERSION}",
                        intention.protocol_version
                    );
                    if let Err(e) =
                        refuse_protocol_version(compose, connection_id, intention.protocol_version)
                    {
                        error!("handshake: failed to tell the client why it was refused: {e}");
                    }
                    // Sending a reason and then shutting down is only safe because this connection
                    // has no output in flight yet. `Shutdown` reaches the proxy as
                    // `PlayerHandle::shutdown`, which calls `kanal::Sender::close`, and that
                    // clears the queue rather than draining it -- so anything still queued for the
                    // player is dropped and the client gets a bare TCP close. A brand-new
                    // connection's writer task is parked in `recv()`, so the reason is handed
                    // straight to it and never sits in the queue. Do not copy this pattern for a
                    // player already in play without fixing the proxy first: ENG-10895.
                    compose.io_buf().shutdown(connection_id);
                    return;
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

/// The chat component a refused client is shown, as a JSON string.
///
/// Split out so the test can build the exact bytes the server should have sent and compare against
/// them: the framing is the thing that needs checking, and it cannot be checked without knowing
/// the body.
fn refusal_reason(client_protocol: i32) -> anyhow::Result<String> {
    let reason = json!({
        "text": format!(
            "This server runs Minecraft {MINECRAFT_VERSION} (protocol {PROTOCOL_VERSION}). Your \
             client speaks protocol {client_protocol}."
        ),
    });
    Ok(serde_json::to_string(&reason)?)
}

/// Tells a client on the wrong protocol why it cannot join.
///
/// The client switches to login state the moment it sends its intention, so a `login_disconnect`
/// is the one refusal it will render; anything sent in handshake state is read against the wrong
/// codec. Compression is negotiated later in login, so this goes out uncompressed: until
/// `login_compression` arrives the client reads frames with no data-length prefix, and a
/// compressed frame would have it read the length byte as the packet id.
fn refuse_protocol_version(
    compose: &Compose,
    connection_id: ConnectionId,
    client_protocol: i32,
) -> anyhow::Result<()> {
    let reason = refusal_reason(client_protocol)?;

    send_uncompressed(
        compose,
        connection_id,
        packet_id::login::clientbound::PacketId::LoginDisconnect.to_raw(),
        &LoginDisconnect { reason: &reason },
    )
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
/// Random per connection, and deliberately not a function of the name. A name
/// hash is what a single-account server wants, and it is what this used to do,
/// but it makes the profile id a second copy of the username: two people who
/// both call themselves `Steve` get one id, and one id is one player as far as
/// the client is concerned. `ClientPacketListener.playerInfoMap` is keyed on
/// the profile id and filled with `putIfAbsent`, so the second `Steve` is
/// dropped from the tab list, renders with the first one's profile, and takes
/// the first one's entry with them when they leave.
///
/// The cost is that an offline id no longer survives a reconnect. Nothing here
/// depends on that: the only thing keyed on a profile id is the skin cache in
/// [`crate::storage::SkinHandler`], which is consulted only for the real Mojang
/// ids of online-mode profiles. When something does need a stable offline
/// identity it should key on the username, which is the thing the player
/// actually controls.
fn offline_uuid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
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
    // this server is offline-mode, so a zero id means mint one here.
    let uuid = if hello.profile_id.0 == 0 {
        offline_uuid()
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

    // Read before the prefab is instantiated because `add_enum` below needs the
    // value, and a nested `world.get` inside the `MetadataPrefabs` borrow would
    // hold two singleton borrows at once.
    let default_gamemode = world.get::<&DefaultGamemode>(|default| default.0);

    // Before the prefab rather than inside the chain below, because this is the
    // id already on the wire in `LoginFinished` and everything after it is
    // decoration. What actually keeps it is the `ConnectionId` term on the
    // auto-uuid observer in `SimModule`; ordering alone does not, because these
    // commands are deferred and that observer's write is appended last. See
    // ENG-10813.
    entity.set(Uuid::from(uuid));

    world.get::<&MetadataPrefabs>(|prefabs| {
        entity
            .is_a(prefabs.player_base)
            .set(Name::from(username.clone()))
            .add(id::<AiTargetable>())
            .set(ImmuneStatus::default())
            .add(id::<Xp>())
            .set_pair::<Prev, _>(Xp::default())
            .add(id::<ChunkSendQueue>())
            .add(id::<Velocity>())
            .set(ChunkPosition::null())
            .set(ActiveAnimation::NONE)
            .set(hyperion_inventory::PlayerInventory::default())
            .set(ConfirmBlockSequences::default())
            .add_enum(default_gamemode)
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

    // Vanilla follows the registries with the tag map, and so must this: a
    // client throws away the tags it loaded from its own packs the moment
    // `update_tags` arrives, so an empty one leaves it with none. The very next
    // thing it does is parse the registry elements it kept, several of which
    // name an item, block or entity-type tag, and one element that fails to
    // parse fails the whole registry load and `finish_configuration` with it.
    // That is the "Network Protocol Error" a real 26.2 client used to hit here.
    send(
        compose,
        connection_id,
        PacketId::UpdateTags.to_raw(),
        &tag_data::VanillaTags,
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

#[cfg(test)]
mod tests {
    use hyperion_proxy_proto::ArchivedServerToProxyMessage;

    use super::*;
    use crate::net::{ProxyId, protocol::Clientbound, tests::two_proxies};

    /// A client on the wrong protocol used to get a `warn!` and a seat on the server. It now gets
    /// a `login_disconnect` it can render, on the connection that asked and on no other.
    ///
    /// This checks the packet reaches the wire rather than that the function returns `Ok`: the
    /// refusal has to be encoded uncompressed and addressed to one connection, and both of those
    /// are wrong in ways `Ok(())` cannot tell you about.
    #[test]
    fn a_refused_client_is_told_why() {
        let (compose, mut zero_rx, mut one_rx) = two_proxies();
        let connection = ConnectionId::new(1, ProxyId::new(0));

        refuse_protocol_version(&compose, connection, 999).unwrap();

        let bytes = zero_rx
            .try_recv()
            .expect("the refused client's proxy must be sent the reason");
        let body = &bytes[size_of::<u64>()..];
        let message = unsafe { rkyv::access_unchecked::<ArchivedServerToProxyMessage<'_>>(body) };

        let ArchivedServerToProxyMessage::Unicast(unicast) = message else {
            panic!("a refusal is addressed to one client, so it goes out as a unicast");
        };
        assert_eq!(
            u64::from(unicast.stream),
            1,
            "the refusal must be addressed to the client that was refused"
        );

        // Byte for byte against the uncompressed encoding, not just "is the id right". The
        // refused client has not been sent `login_compression` yet, so it reads frames with no
        // data-length prefix; a compressed frame is one byte longer and it would read that length
        // byte as the packet id. Checking a single byte cannot tell the two apart, because both
        // framings put a zero at that offset and `LoginDisconnect` is itself id 0.
        let payload = unicast.data.as_ref();
        let reason = refusal_reason(999).unwrap();
        let expected = compose
            .io_buf()
            .encode_packet_no_compression(Clientbound::new(
                packet_id::login::clientbound::PacketId::LoginDisconnect.to_raw(),
                &LoginDisconnect { reason: &reason },
            ))
            .unwrap();
        assert_eq!(
            payload,
            expected.as_ref(),
            "the refusal must be the uncompressed login_disconnect frame, byte for byte"
        );

        // The reason is what a player actually sees, so it has to name both versions.
        let text = String::from_utf8_lossy(payload);
        assert!(
            text.contains(MINECRAFT_VERSION) && text.contains(&PROTOCOL_VERSION.to_string()),
            "the reason must say what this server speaks, got: {text}"
        );
        assert!(
            text.contains("999"),
            "the reason must say what the client spoke, got: {text}"
        );

        assert!(
            crate::net::tests::next_variant(&mut one_rx).is_none(),
            "a proxy the refused client is not on must hear nothing"
        );
    }
}
