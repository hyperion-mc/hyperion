//! Communication to a proxy which forwards packets to the players.

use std::{io::Cursor, net::SocketAddr, process::Command, sync::Arc};

use bevy::prelude::*;
use bytes::{Buf, BytesMut};
use dashmap::DashMap;
use hyperion_proto::ArchivedProxyToServerMessage;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{error, info, warn};

use crate::{
    ConnectionId, PacketDecoder,
    command_channel::CommandChannel,
    ingress::Disconnect,
    runtime::AsyncRuntime,
    simulation::{EgressComm, StreamLookup, packet_state},
};

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

async fn inner(
    socket: SocketAddr,
    mut server_to_proxy: tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    command_channel: CommandChannel,
) {
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

    tokio::spawn(
        async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                socket.set_nodelay(true).unwrap();

                let addr = socket.peer_addr().unwrap();

                info!("Proxy connection established on {addr}");

                let (read, mut write) = socket.into_split();

                let proxy_writer_task = tokio::spawn(async move {
                    while let Some(bytes) = server_to_proxy.recv().await {
                        if write.write_all(&bytes).await.is_err() {
                            error!("error writing to proxy");
                            return server_to_proxy;
                        }
                    }

                    warn!("proxy shut down");

                    server_to_proxy
                });

                let command_channel = command_channel.clone();
                tokio::spawn(async move {
                    let mut reader = ProxyReader::new(read);

                    loop {
                        let buffer = match reader.next_server_packet_buffer().await {
                            Ok(message) => message,
                            Err(err) => {
                                error!("failed to process packet {err:?}");
                                return;
                            }
                        };

                        let result = unsafe {
                            rkyv::access_unchecked::<ArchivedProxyToServerMessage<'_>>(&buffer)
                        };

                        match result {
                            ArchivedProxyToServerMessage::PlayerConnect(message) => {
                                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);

                                command_channel.push(move |world: &mut World| {
                                    let player = world
                                        .spawn((
                                            ConnectionId::new(stream),
                                            // hyperion_inventory::PlayerInventory::default(),
                                            // ConfirmBlockSequences::default(),
                                            packet_state::Handshake(()),
                                            // ActiveAnimation::NONE,
                                            PacketDecoder::default(),
                                        ))
                                        .id();
                                    world
                                        .get_resource_mut::<StreamLookup>()
                                        .expect("StreamLookup resource should exist")
                                        .insert(stream, player);
                                });
                            }
                            ArchivedProxyToServerMessage::PlayerDisconnect(message) => {
                                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);
                                command_channel.push(move |world: &mut World| {
                                    let player = world
                                        .get_resource_mut::<StreamLookup>()
                                        .expect("StreamLookup resource should exist")
                                        .remove(&stream)
                                        .expect(
                                            "player from PlayerDisconnect must exist in the \
                                             stream lookup map",
                                        );
                                    world.trigger_targets(Disconnect(()), player);
                                });
                            }
                            ArchivedProxyToServerMessage::PlayerPackets(message) => {
                                let Ok(stream) = rkyv::deserialize::<u64, !>(&message.stream);

                                // TODO: avoid this allocation
                                let data = message.data.to_vec();

                                command_channel.push(move |world: &mut World| {
                                    let lookup = world
                                        .get_resource_mut::<StreamLookup>()
                                        .expect("StreamLookup resource should exist");
                                    let player = *lookup.get(&stream).expect(
                                        "player from PlayerPackets must exist in the stream \
                                         lookup map",
                                    );

                                    let mut player = world
                                        .get_entity_mut(player)
                                        .expect("player from StreamLookup must be alive");
                                    let mut decoder = player
                                        .get_mut::<PacketDecoder>()
                                        .expect("all players must have a PacketDecodewr");

                                    decoder.shift_excess();
                                    decoder.queue_slice(&data);
                                });
                            }
                        }
                    }
                });

                // todo: handle player disconnects on proxy shut down
                // Ideally, we should design for there being multiple proxies,
                // and all proxies should store all the players on them.
                // Then we can disconnect all those players related to that proxy.
                server_to_proxy = proxy_writer_task.await.unwrap();
            }
        }, // .instrument(info_span!("proxy reader")),
    );
}

/// Initializes proxy communications.
#[must_use]
pub fn init_proxy_comms(
    runtime: &AsyncRuntime,
    command_channel: CommandChannel,
    socket: SocketAddr,
) -> EgressComm {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    runtime.spawn(inner(socket, rx, command_channel));
    EgressComm::from(tx)
}

#[derive(Debug)]
struct ProxyReader {
    server_read: tokio::net::tcp::OwnedReadHalf,
    buffer: BytesMut,
}

impl ProxyReader {
    pub fn new(server_read: tokio::net::tcp::OwnedReadHalf) -> Self {
        Self {
            server_read,
            buffer: BytesMut::with_capacity(1024 * 1024),
        }
    }

    // #[instrument]
    pub async fn next_server_packet_buffer(&mut self) -> anyhow::Result<BytesMut> {
        let len = loop {
            if !self.buffer.is_empty() {
                let mut cursor = Cursor::new(&self.buffer);

                // todo: handle invalid varint
                if let Ok(len) =
                    byteorder::ReadBytesExt::read_u64::<byteorder::BigEndian>(&mut cursor)
                {
                    self.buffer.advance(usize::try_from(cursor.position())?);
                    break usize::try_from(len)?;
                }
            }

            self.server_read.read_buf(&mut self.buffer).await?;
        };

        // todo: this needed?
        self.buffer.reserve(len);

        while self.buffer.len() < len {
            self.server_read.read_buf(&mut self.buffer).await?;
        }

        let buffer = self.buffer.split_to(len);

        Ok(buffer)
    }
}
