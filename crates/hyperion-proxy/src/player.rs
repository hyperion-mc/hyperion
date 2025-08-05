//! Player connection handling and packet processing.

use std::io::IoSlice;

use arrayvec::ArrayVec;
use bytes::Bytes;
use hyperion_proto::{
    PlayerConnect, PlayerDisconnect, PlayerDisconnectReason, PlayerPackets, ProxyToServerMessage,
};
use rkyv::ser::allocator::Arena;
use rustc_hash::FxBuildHasher;
use tokio::{
    io::{AsyncReadExt, AsyncWrite},
    task::JoinHandle,
};
use tracing::{info, info_span, instrument, warn};

use crate::{
    ShutdownType, data::PlayerHandle, server_sender::ServerSender, util::AsyncWriteVectoredExt,
};

/// Default buffer size for reading player packets, set to 8 KiB.
const DEFAULT_READ_BUFFER_SIZE: usize = 8 * 1024;

/// Initiates a player connection handler, managing both incoming and outgoing packet streams.
///
/// This function sets up two asynchronous tasks:
/// 1. A reader task that processes incoming packets from the player.
/// 2. A writer task that sends outgoing packets to the player.
///
/// It also handles player disconnection and shutdown scenarios.
#[instrument(skip_all, fields(player_id = player_id))]
pub fn initiate_player_connection(
    socket: impl tokio::io::AsyncRead + AsyncWrite + Send + 'static,
    mut shutdown_signal: tokio::sync::watch::Receiver<Option<ShutdownType>>,
    player_id: u64,
    incoming_packet_receiver: kanal::AsyncReceiver<Bytes>,
    server_sender: ServerSender,
    player_registry: &'static papaya::HashMap<u64, PlayerHandle, FxBuildHasher>,
) -> JoinHandle<()> {
    let span = info_span!("player_connection", player_id);
    let _enter = span.enter();

    info!("Initiating player connection");
    let (socket_reader, socket_writer) = tokio::io::split(socket);

    let mut socket_reader = Box::pin(socket_reader);
    let mut socket_writer = Box::pin(socket_writer);

    // Task for handling incoming packets (player -> proxy)
    let mut packet_reader_task = tokio::spawn({
        let server_sender = server_sender.clone();
        async move {
            let mut read_buffer = Vec::new();
            let player_stream_id = player_id;

            let connect = rkyv::to_bytes::<rkyv::rancor::Error>(
                &ProxyToServerMessage::PlayerConnect(PlayerConnect {
                    stream: player_stream_id,
                }),
            )
            .unwrap();

            if let Err(e) = server_sender.send(connect).await {
                warn!("failed to send player connect to server: {e}");
                return;
            }

            let mut arena = Arena::new();

            loop {
                // Ensure the buffer has enough capacity
                read_buffer.reserve(DEFAULT_READ_BUFFER_SIZE);

                let bytes_read = match socket_reader.read_buf(&mut read_buffer).await {
                    Ok(n) => n,
                    Err(e) => {
                        warn!("Error reading from player: {e:?}");
                        return;
                    }
                };

                if bytes_read == 0 {
                    warn!("End of stream reached for player");
                    return;
                }

                let player_packets = ProxyToServerMessage::PlayerPackets(PlayerPackets {
                    stream: player_id,
                    data: &read_buffer,
                });

                let aligned_vec = rkyv::api::high::to_bytes_with_alloc::<_, rkyv::rancor::Error>(
                    &player_packets,
                    arena.acquire(),
                )
                .unwrap();

                read_buffer.clear();

                if let Err(e) = server_sender.send(aligned_vec).await {
                    warn!("Error forwarding player packets to server: {e:?}");
                    return;
                }
            }
        }
    });

    // Task for handling outgoing packets (proxy -> player)
    let mut packet_writer_task = tokio::spawn(async move {
        while let Ok(outgoing_packet) = incoming_packet_receiver.recv().await {
            let mut bytes = ArrayVec::<_, 16>::new();
            bytes.push(outgoing_packet);

            // Try reading more bytes from the channel
            while bytes.remaining_capacity() > 0 {
                let Ok(Some(outgoing_packet)) = incoming_packet_receiver.try_recv() else {
                    break;
                };
                bytes.push(outgoing_packet);
            }

            // Convert the bytes into slices
            let mut slices = ArrayVec::<_, 16>::new();
            for slice in &bytes {
                slices.push(IoSlice::new(slice));
            }

            if let Err(e) = socket_writer.write_vectored_all(&mut slices).await {
                warn!("Error writing packets to player: {e:?}");
                return;
            }
        }
    });

    tokio::task::spawn(async move {
        let shutdown_received = async move {
            shutdown_signal.wait_for(Option::is_some).await.unwrap();
        };

        tokio::select! {
            () = shutdown_received => {
                info!("Shutting down player connection due to server shutdown");
                packet_reader_task.abort();
                packet_writer_task.abort();
            },
            _ = &mut packet_writer_task => {
                info!("Player disconnected because writer task finished: {player_id:?}");
                packet_reader_task.abort();

                let disconnect = rkyv::to_bytes::<rkyv::rancor::Error>(
                    &ProxyToServerMessage::PlayerDisconnect(PlayerDisconnect {
                        stream: player_id,
                        reason: PlayerDisconnectReason::LostConnection,
                    }),
                ).unwrap();

                if let Err(e) = server_sender.send(disconnect).await {
                    warn!("failed to send player disconnect to server: {e}");
                }
            },
            _ = &mut packet_reader_task => {
                info!("Player disconnected because reader task finished: {player_id:?}");
                packet_writer_task.abort();


                let disconnect = rkyv::to_bytes::<rkyv::rancor::Error>(
                    &ProxyToServerMessage::PlayerDisconnect(PlayerDisconnect {
                        stream: player_id,
                        reason: PlayerDisconnectReason::LostConnection,
                    })).unwrap();

                if let Err(e) = server_sender.send(disconnect).await {
                    warn!("failed to send player disconnect to server: {e}");
                }
            }
        }

        let map_ref = player_registry.pin();
        map_ref.remove(&player_id);
    })
}
