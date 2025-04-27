//! Player connection handling and packet processing.

use std::io::IoSlice;

use hyperion_proto::{
    ChunkPosition, PlayerConnect, PlayerDisconnect, PlayerDisconnectReason, PlayerPackets,
    ProxyToServerMessage,
};
use rkyv::ser::allocator::Arena;
use rustc_hash::FxBuildHasher;
use tokio::{
    io::{AsyncReadExt, AsyncWrite},
    task::JoinHandle,
};
use tracing::{info, info_span, instrument, warn};

use crate::{
    ShutdownType,
    cache::ExclusionsManager,
    data::{OrderedBytes, PlayerHandle},
    server_sender::ServerSender,
    util::AsyncWriteVectoredExt,
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
    incoming_packet_receiver: kanal::AsyncReceiver<OrderedBytes>,
    server_sender: ServerSender,
    player_registry: &'static papaya::HashMap<u64, PlayerHandle, FxBuildHasher>,
    player_positions: &'static papaya::HashMap<u64, ChunkPosition, FxBuildHasher>,
) -> JoinHandle<()> {
    let span = info_span!("player_connection", player_id);
    let _enter = span.enter();

    info!("Initiating player connection");
    let (socket_reader, socket_writer) = tokio::io::split(socket);

    let mut socket_reader = Box::pin(socket_reader);
    let socket_writer = Box::pin(socket_writer);

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
        let mut packet_writer = PlayerPacketWriter::new(socket_writer, player_id);

        while let Ok(outgoing_packet) = incoming_packet_receiver.recv().await {
            if outgoing_packet.is_flush() {
                let time_start = std::time::Instant::now();
                if let Err(e) = packet_writer.flush_pending_packets().await {
                    warn!("Error flushing packets to player: {e:?}");
                    return;
                }
                let duration = time_start.elapsed();
                if duration > std::time::Duration::from_millis(50) {
                    warn!("flushed packets to player in {duration:?}");
                }
            } else {
                packet_writer.enqueue_packet(outgoing_packet);
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

        let map_ref = player_positions.pin();
        map_ref.remove(&player_id);
    })
}

/// Manages the writing of packets to a player's connection.
struct PlayerPacketWriter<W> {
    writer: W,
    player_id: u64,
    pending_packets: Vec<OrderedBytes>,
    io_vecs: Vec<IoSlice<'static>>,
}

impl<W: AsyncWrite + Unpin> PlayerPacketWriter<W> {
    /// Creates a new [`PlayerPacketWriter`] instance.
    const fn new(writer: W, player_id: u64) -> Self {
        Self {
            writer,
            player_id,
            pending_packets: Vec::new(),
            io_vecs: vec![],
        }
    }

    /// Adds a packet to the queue for writing.
    fn enqueue_packet(&mut self, packet: OrderedBytes) {
        self.pending_packets.push(packet);
    }

    /// Flushes all pending packets to the TCP writer.
    #[instrument(skip(self), fields(player_id = ?self.player_id), level = "trace")]
    async fn flush_pending_packets(&mut self) -> anyhow::Result<()> {
        for iovec in prepare_io_vectors(&mut self.pending_packets, self.player_id) {
            // extend lifetime of iovecs so we can reuse the io_vecs Vec
            let iovec = unsafe { std::mem::transmute::<IoSlice<'_>, IoSlice<'static>>(iovec) };
            self.io_vecs.push(iovec);
        }

        if self.io_vecs.is_empty() {
            self.pending_packets.clear();
            return Ok(());
        }

        #[cfg(debug_assertions)]
        {
            // assert none are empty
            for iovec in &self.io_vecs {
                debug_assert!(!iovec.is_empty());
            }
        }

        self.writer.write_vectored_all(&mut self.io_vecs).await?;
        self.pending_packets.clear();
        self.io_vecs.clear();

        Ok(())
    }
}

/// Prepares IO vectors from the queue of ordered bytes, applying necessary exclusions.
fn prepare_io_vectors(
    packet_queue: &mut [OrderedBytes],
    player_id: u64,
) -> impl Iterator<Item = IoSlice<'_>> + '_ {
    packet_queue.sort_unstable_by_key(|packet| packet.order);

    packet_queue.iter_mut().flat_map(move |packet| {
        let packet_data = packet.data.as_ref();
        apply_exclusions(
            packet.offset,
            packet_data,
            packet.exclusions.as_deref(),
            player_id,
        )
    })
}

/// Generates IO vectors to right given ranges of data that should be excluded.
fn apply_exclusions<'a>(
    offset: u32,
    packet_data: &'a [u8],
    exclusions: Option<&'a ExclusionsManager>,
    player_id: u64,
) -> impl Iterator<Item = IoSlice<'a>> + 'a {
    gen move {
        let mut current_offset = 0;

        if let Some(exclusions) = exclusions {
            // Process exclusions in reverse order
            // We reverse the order because ExclusionIterator returns ranges starting from
            // the most recent (tail) node in the linked list structure (see @cache.rs).
            // By reversing, we process exclusions from the start of the packet to the end.
            let mut exclusion_ranges: heapless::Vec<_, 16> = heapless::Vec::new();

            for mut range in exclusions.exclusions_for_player(player_id) {
                range.move_left(offset);

                let Some(range) = range.clamp(&(0..packet_data.len())) else {
                    continue;
                };

                exclusion_ranges.push(range).unwrap();
            }

            exclusion_ranges.reverse();

            // Iterate through the reversed ranges to properly exclude sections
            // from the beginning to the end of the packet
            for range in exclusion_ranges {
                let included_slice = &packet_data[current_offset..range.start];
                yield IoSlice::new(included_slice);
                current_offset = range.end;
            }
        }

        // Write remaining data or full packet if no exclusions
        if current_offset == 0 {
            yield IoSlice::new(packet_data);
        } else {
            let remaining_slice = &packet_data[current_offset..];
            yield IoSlice::new(remaining_slice);
        }
    }
    .filter(|iovec| !iovec.is_empty())
}

trait RangeExt {
    fn move_left(&mut self, offset: u32);
    fn clamp(&self, other: &std::ops::Range<usize>) -> Option<std::ops::Range<usize>>;
}

impl RangeExt for std::ops::Range<usize> {
    fn move_left(&mut self, offset: u32) {
        if offset > 0 {
            let offset = offset as usize;
            if self.start >= offset {
                self.start -= offset;
            } else {
                self.start = 0;
            }
            if self.end >= offset {
                self.end -= offset;
            } else {
                self.end = 0;
            }
        }
    }

    fn clamp(&self, other: &std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
        if self.end <= other.start || self.start >= other.end {
            None
        } else {
            Some(Self {
                start: self.start.max(other.start),
                end: self.end.min(other.end),
            })
        }
    }
}
