//! Communication to a proxy which forwards packets to the players.

use std::{
    net::SocketAddr,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::entity::RemoveEntities,
};
use hyperion_proto::ArchivedProxyToServerMessage;
use hyperion_utils::EntityExt;
use rustc_hash::FxHashMap;
use rustls::{
    RootCertStore,
    server::{ServerConfig, WebPkiClientVerifier},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

use crate::{
    ConnectionId, Crypto, PacketDecoder,
    command_channel::CommandChannel,
    net::{Channel, ChannelId, Compose, IoBuf, ProxyId, protocol::Clientbound},
    runtime::AsyncRuntime,
    simulation::{EgressComm, PacketState, StreamLookup, event::RequestSubscribeChannelPackets},
    storage::Events,
};

// TODO: Determine a better default
const DEFAULT_FRAGMENT_SIZE: usize = 4096;

/// Removes a disconnected proxy's channel from the compose egress comms list.
fn remove_proxy_from_compose(world: &World, proxy_id: ProxyId) {
    world.get::<&mut Compose>(|compose| {
        if compose.io_buf_mut().remove_proxy(proxy_id).is_none() {
            error!("failed to remove proxy from compose egress comms");
        }
    });
}

fn get_pid_from_port(port: u16) -> Result<Option<u32>, std::io::Error> {
    let output = if cfg!(target_os = "windows") {
        // todo: untested
        Command::new("cmd")
            .args(["/C", &format!("netstat -ano | findstr :{port}")])
            .output()?
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(format!("lsof -i :{port} -t"))
            .output()?
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = stdout.lines().next().and_then(|line| line.parse().ok());

    Ok(pid)
}

async fn handle_proxy_messages(
    read: impl AsyncRead + Unpin,
    command_channel: CommandChannel,
    proxy_id: ProxyId,
) {
    let mut reader = ProxyReader::new(read);
    let mut player_packet_sender: FxHashMap<u64, packet_channel::Sender> = FxHashMap::default();

    // Process packets
    loop {
        let buffer = match reader.next_server_packet_buffer().await {
            Ok(message) => message,
            Err(err) => {
                match err.downcast::<std::io::Error>() {
                    Ok(io_err) => match io_err.kind() {
                        std::io::ErrorKind::UnexpectedEof => {
                            warn!("proxy closed proxy to server connection");
                        }
                        kind => {
                            error!("closing proxy connection due to an i/o error: {kind}");
                        }
                    },
                    Err(err) => {
                        error!("closing proxy connection due to an unexpected error: {err:?}");
                    }
                }
                break;
            }
        };

        let result = unsafe { rkyv::access_unchecked::<ArchivedProxyToServerMessage<'_>>(buffer) };

        match result {
            ArchivedProxyToServerMessage::PlayerConnect(message) => {
                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);

                let (sender, receiver) = packet_channel::channel(DEFAULT_FRAGMENT_SIZE);
                if player_packet_sender.insert(stream, sender).is_some() {
                    error!(
                        "PlayerConnect: player with same stream id already exists in \
                         player_packet_sender"
                    );
                }

                command_channel.push(move |world: &World| {
                    let player = world
                        .entity()
                        .set(ConnectionId::new(stream, proxy_id))
                        .add_enum(PacketState::Handshake)
                        .set(PacketDecoder::default())
                        .set(receiver)
                        .id();

                    world.get::<&mut StreamLookup>(|lookup| {
                        lookup.insert(stream, player);
                    });
                });
            }
            ArchivedProxyToServerMessage::PlayerDisconnect(message) => {
                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);

                if player_packet_sender.remove(&stream).is_none() {
                    error!(
                        "PlayerDisconnect: no player with stream id exists in player_packet_sender"
                    );
                }

                command_channel.push(move |world: &World| {
                    let player = world.get::<&mut StreamLookup>(|lookup| {
                        lookup.remove(&stream).expect(
                            "player from PlayerDisconnect must exist in the stream lookup map",
                        )
                    });

                    world.entity_from_id(player).destruct();
                });
            }
            ArchivedProxyToServerMessage::PlayerPackets(message) => {
                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);

                let Some(sender) = player_packet_sender.get_mut(&stream) else {
                    error!(
                        "PlayerPackets: no player with stream id exists in player_packet_sender"
                    );
                    continue;
                };

                if let Err(e) = sender.send(&message.data) {
                    use packet_channel::SendError;
                    let needs_shutdown = match e {
                        SendError::ZeroLengthPacket => {
                            warn!("A player sent an illegal zero-length packet, disconnecting");
                            true
                        }
                        SendError::TooLargePacket => {
                            warn!("A player sent a packet that is too large, disconnecting");
                            true
                        }
                        SendError::AlreadyClosed => false,
                    };
                    if needs_shutdown {
                        command_channel.push(move |world: &World| {
                            world.get::<&Compose>(|compose| {
                                compose
                                    .io_buf()
                                    .shutdown(ConnectionId::new(stream, proxy_id));
                            });
                        });
                    }
                }
            }
            ArchivedProxyToServerMessage::RequestSubscribeChannelPackets(message) => {
                let channels =
                    match rkyv::deserialize::<Box<[u32]>, rkyv::rancor::Error>(&message.channels) {
                        Ok(channels) => channels,
                        Err(e) => {
                            error!(
                                "RequestSubscribeChannelPackets: failed to deserialize channels: \
                                 {e}"
                            );
                            continue;
                        }
                    };

                command_channel.push(move |world: &World| {
                    world.get::<&Events>(|events| {
                        for channel_id in channels {
                            let channel = Entity::from_minecraft_id(bytemuck::cast(channel_id));
                            let view = world.entity_from_id(channel);

                            if !view.is_alive() || !view.has(id::<Channel>()) {
                                error!(
                                    "RequestSubscribeChannelPackets: channel id is invalid: \
                                     {channel_id}"
                                );
                                continue;
                            }

                            events.push(RequestSubscribeChannelPackets(channel), world);
                        }
                    });
                });
            }
        }
    }

    // Disconnect all players that were connected through this proxy
    command_channel.push(move |world: &World| {
        let mut players_to_remove = Vec::new();

        world
            .new_query::<&ConnectionId>()
            .each_entity(|entity, connection_id| {
                if connection_id.proxy_id() == proxy_id {
                    players_to_remove.push(entity.id());
                }
            });

        for player in players_to_remove {
            world.entity_from_id(player).destruct();
        }
    });
}

async fn inner(socket: SocketAddr, crypto: Crypto, command_channel: CommandChannel) {
    let listener = match tokio::net::TcpListener::bind(socket).await {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let error_msg = format!(
                "Failed to bind to address {socket}: Already in use. Is another process using \
                 this port?"
            );
            let port = socket.port();

            match get_pid_from_port(port) {
                Ok(Some(pid)) => {
                    let error_msg =
                        format!("{error_msg}\nAlready in use by process with PID {pid}");
                    panic!("{error_msg}");
                }
                Ok(None) => {
                    panic!("{error_msg} for port {port}");
                }
                Err(e) => {
                    let error_msg = format!("{error_msg}\n{e}");
                    panic!("{error_msg}");
                }
            }
        }
        Err(e) => panic!("Failed to bind to address {socket}: {e}"),
    };

    let root_cert_store = Arc::new(RootCertStore {
        roots: vec![
            webpki::anchor_from_trusted_cert(&crypto.root_ca_cert)
                .unwrap()
                .to_owned(),
        ],
    });

    let config = ServerConfig::builder()
        .with_client_cert_verifier(
            WebPkiClientVerifier::builder(root_cert_store)
                .build()
                .unwrap(),
        )
        .with_single_cert(vec![crypto.cert, crypto.root_ca_cert], crypto.key)
        .unwrap();

    let acceptor = TlsAcceptor::from(Arc::new(config));

    tokio::spawn(
        async move {
            let next_proxy_id = Arc::new(AtomicU64::new(0));

            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(e) => {
                        error!("failed to accept proxy connection: {e}");
                        continue;
                    }
                };

                // A peer that disconnects between accept and here makes this
                // fail with EINVAL. Unwrapping killed the whole accept loop, so
                // any half-open connection took the server's proxy listener
                // down with it.
                if let Err(e) = socket.set_nodelay(true) {
                    error!("failed to accept proxy connection: set_nodelay failed: {e}");
                    continue;
                }

                let addr = match socket.peer_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        error!("failed to accept proxy connection: peer addr failed: {e}");
                        continue;
                    }
                };

                let command_channel = command_channel.clone();
                let next_proxy_id = next_proxy_id.clone();
                let stream = acceptor.accept(socket);

                tokio::spawn(async move {
                    let stream = match stream.await {
                        Ok(stream) => stream,
                        Err(e) => {
                            error!(
                                "failed to accept proxy connection from {addr}: tls accept \
                                 failed: {e}"
                            );
                            return;
                        }
                    };

                    info!("Proxy connection established on {addr}");

                    let (read, mut write) = tokio::io::split(stream);

                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    let egress_comm = EgressComm::from(tx.clone());
                    let proxy_id = ProxyId::new(next_proxy_id.fetch_add(1, Ordering::Relaxed));

                    command_channel.push(move |world: &World| {
                        world.get::<&mut Compose>(|compose| {
                            compose.io_buf_mut().add_proxy(proxy_id, egress_comm);
                        });
                    });

                    let command_channel_clone = command_channel.clone();
                    tokio::spawn(async move {
                        // Send the bytes from the channel to the proxy
                        while let Some(bytes) = rx.recv().await {
                            if write.write_all(&bytes).await.is_err() {
                                error!("error writing to proxy");
                                break;
                            }
                        }

                        warn!("proxy shut down");

                        command_channel_clone.push(move |world: &World| {
                            remove_proxy_from_compose(world, proxy_id);

                            // Explicitly close this receiver. This ensures that the channel isn't
                            // closed before this, which would lead to an error on the sender side
                            // of Compose.
                            rx.close();
                        });
                    });

                    command_channel.push(move |world: &World| {
                        // Let the proxy know about all packet channels that exist at the moment
                        let query = world.query::<()>().with(id::<Channel>()).build();

                        world.get::<&Compose>(|compose| {
                            query.each_entity(|channel, ()| {
                                let packet = RemoveEntities(vec![channel.id().minecraft_id()]);

                                let packet_buf = compose
                                    .io_buf()
                                    .encode_packet(
                                        Clientbound::new(
                                            PacketId::RemoveEntities.to_raw(),
                                            &packet,
                                        ),
                                        compose,
                                    )
                                    .unwrap();

                                tx.send(IoBuf::encode_proxy_message(
                                    &hyperion_proto::ServerToProxyMessage::AddChannel(
                                        hyperion_proto::AddChannel {
                                            channel_id: ChannelId::from(channel).inner(),
                                            unsubscribe_packets: &packet_buf,
                                        },
                                    ),
                                ))
                                .unwrap();
                            });
                        });
                    });

                    tokio::spawn(handle_proxy_messages(
                        read,
                        command_channel.clone(),
                        proxy_id,
                    ));
                });
            }
        }, // .instrument(info_span!("proxy reader")),
    );
}

/// Initializes proxy communications.
pub fn init_proxy_comms(
    runtime: &AsyncRuntime,
    command_channel: CommandChannel,
    socket: SocketAddr,
    crypto: Crypto,
) {
    runtime.spawn(inner(socket, crypto, command_channel));
}

#[derive(Debug)]
struct ProxyReader<R> {
    server_read: R,
    buffer: Vec<u8>,
}

impl<R> ProxyReader<R>
where
    R: AsyncRead + Unpin,
{
    pub fn new(server_read: R) -> Self {
        Self {
            server_read,
            buffer: vec![0; 1024 * 1024],
        }
    }

    // #[instrument]
    pub async fn next_server_packet_buffer(&mut self) -> anyhow::Result<&[u8]> {
        let mut len = [0u8; 8];
        self.server_read.read_exact(&mut len).await?;
        let len = u64::from_be_bytes(len);
        let len = usize::try_from(len)?;

        if len > self.buffer.len() {
            self.buffer.resize(len, 0);
        }

        let buffer = &mut self.buffer[..len];

        self.server_read.read_exact(buffer).await?;

        Ok(buffer)
    }
}
