//! All the networking related code.

use std::{
    cell::{Cell, RefCell},
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
};

use bevy::prelude::*;
use bumpalo::Bump;
use byteorder::WriteBytesExt;
use bytes::{Bytes, BytesMut};
pub use decoder::PacketDecoder;
use derive_more::Deref;
use glam::I16Vec2;
use hyperion_proto::{ChunkPosition, ServerToProxyMessage};
use hyperion_utils::LifetimeTracker;
use libdeflater::CompressionLvl;
use rkyv::util::AlignedVec;
use thread_local::ThreadLocal;

use crate::{
    Global, PacketBundle, Scratch,
    net::encoder::{PacketEncoder, append_packet_without_compression},
};

pub mod agnostic;
pub mod decoder;
pub mod encoder;
pub mod packet_channel;
pub mod packets;
pub mod proxy;

/// The Minecraft protocol version this library currently targets.
pub const PROTOCOL_VERSION: i32 = 763;

/// The maximum number of bytes that can be sent in a single packet.
pub const MAX_PACKET_SIZE: usize = valence_protocol::MAX_PACKET_SIZE as usize;

/// The stringified name of the Minecraft version this library currently
/// targets.
pub const MINECRAFT_VERSION: &str = "1.20.1";

/// A unique identifier for a client connection
///
/// Each `ConnectionId` represents an active network connection between the server and a client,
/// corresponding to a single player session. The ID is used throughout the networking
/// system to:
///
/// - Route packets to a specific client
/// - Target or exclude specific clients in broadcast operations
/// - Track connection state through the proxy layer
///
/// Note: Connection IDs are managed internally by the networking system and should be obtained
/// through the appropriate connection establishment handlers rather than created directly.
#[derive(Component, Copy, Clone, Debug)]
pub struct ConnectionId {
    /// The underlying unique identifier for this connection.
    /// This value is guaranteed to be unique among all active connections.
    stream_id: u64,
}

impl ConnectionId {
    /// Creates a new connection ID with the specified stream identifier.
    ///
    /// This is an internal API used by the connection management system.
    /// External code should obtain connection IDs through the appropriate
    /// connection handlers.
    #[must_use]
    pub const fn new(stream_id: u64) -> Self {
        Self { stream_id }
    }

    /// Returns the underlying stream identifier.
    ///
    /// This method is primarily used by internal networking code to interact
    /// with the proxy layer. Most application code should work with the
    /// `ConnectionId` type directly rather than the raw ID.
    #[must_use]
    pub const fn inner(self) -> u64 {
        self.stream_id
    }
}

/// A singleton that can be used to compose and encode packets.
#[derive(Resource)]
pub struct Compose {
    compression_lvl: CompressionLvl,
    compressor: ThreadLocal<RefCell<libdeflater::Compressor>>,
    scratch: ThreadLocal<RefCell<Scratch>>,
    global: Global,
    io_buf: IoBuf,
    pub bump: ThreadLocal<Bump>,
    pub bump_tracker: LifetimeTracker,
}

#[must_use]
pub struct DataBundle<'a> {
    compose: &'a Compose,
    data: BytesMut,
}

impl<'a> DataBundle<'a> {
    pub fn new(compose: &'a Compose) -> Self {
        Self {
            compose,
            data: BytesMut::new(),
        }
    }

    pub fn add_packet(&mut self, pkt: impl PacketBundle) -> anyhow::Result<()> {
        let data = self.compose.io_buf.encode_packet(pkt, self.compose)?;
        // todo: test to see if this ever actually unsplits
        self.data.unsplit(data);
        Ok(())
    }

    pub fn add_raw(&mut self, raw: &[u8]) {
        self.data.extend_from_slice(raw);
    }

    pub fn unicast(&self, stream: ConnectionId) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose.io_buf.unicast_raw(&self.data, stream);
        Ok(())
    }

    // todo: use builder pattern for excluding
    pub fn broadcast_local(&self, center: I16Vec2) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose
            .io_buf
            .broadcast_local_raw(&self.data, center, 0);
        Ok(())
    }
}

impl Compose {
    #[must_use]
    pub fn new(compression_lvl: CompressionLvl, global: Global, io_buf: IoBuf) -> Self {
        Self {
            compression_lvl,
            compressor: ThreadLocal::new(),
            scratch: ThreadLocal::new(),
            global,
            io_buf,
            bump: ThreadLocal::new(),
            bump_tracker: LifetimeTracker::default(),
        }
    }

    #[must_use]
    #[expect(missing_docs)]
    pub const fn global(&self) -> &Global {
        &self.global
    }

    #[expect(missing_docs)]
    pub const fn global_mut(&mut self) -> &mut Global {
        &mut self.global
    }

    /// Broadcast globally to all players
    ///
    /// See <https://github.com/andrewgazelka/hyperion-proto/blob/main/src/server_to_proxy.proto#L17-L22>
    pub const fn broadcast<'a, P>(&'a self, packet: P) -> Broadcast<'a, P>
    where
        P: PacketBundle,
    {
        Broadcast {
            packet,
            compose: self,
            exclude: 0,
        }
    }

    #[must_use]
    #[expect(missing_docs)]
    pub const fn io_buf(&self) -> &IoBuf {
        &self.io_buf
    }

    #[expect(missing_docs)]
    pub const fn io_buf_mut(&mut self) -> &mut IoBuf {
        &mut self.io_buf
    }

    /// Broadcast a packet within a certain region.
    ///
    /// See <https://github.com/andrewgazelka/hyperion-proto/blob/main/src/server_to_proxy.proto#L17-L22>
    pub const fn broadcast_local<'a, P>(
        &'a self,
        packet: P,
        center: I16Vec2,
    ) -> BroadcastLocal<'a, P>
    where
        P: PacketBundle,
    {
        BroadcastLocal {
            packet,
            compose: self,
            exclude: 0,
            center: ChunkPosition {
                x: center.x,
                z: center.y,
            },
        }
    }

    /// Send a packet to a single player.
    pub fn unicast<P>(&self, packet: P, stream_id: ConnectionId) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        Unicast {
            packet,
            stream_id,
            compose: self,
            // todo: Should we have this true by default, or is there a better way?
            // Or a better word for no_compress, or should we just use negative field names?
            compress: true,
        }
        .send()
    }

    /// Send a packet to a single player without compression.
    pub fn unicast_no_compression<P>(
        &self,
        packet: &P,
        stream_id: ConnectionId,
    ) -> anyhow::Result<()>
    where
        P: valence_protocol::Packet + valence_protocol::Encode,
    {
        Unicast {
            packet,
            stream_id,
            compose: self,
            compress: false,
        }
        .send()
    }

    #[must_use]
    #[allow(clippy::missing_const_for_fn, reason = "this is a false positive")]
    pub(crate) fn encoder(&self) -> PacketEncoder {
        let threshold = self.global.shared.compression_threshold;
        PacketEncoder::new(threshold)
    }

    /// Obtain a thread-local scratch buffer.
    #[must_use]
    pub fn scratch(&self) -> &RefCell<Scratch> {
        self.scratch.get_or_default()
    }

    /// Obtain a thread-local [`libdeflater::Compressor`]
    #[must_use]
    pub fn compressor(&self) -> &RefCell<libdeflater::Compressor> {
        self.compressor
            .get_or(|| RefCell::new(libdeflater::Compressor::new(self.compression_lvl)))
    }

    #[must_use]
    pub(crate) fn bump(&self) -> &Bump {
        self.bump.get_or_default()
    }

    pub fn clear_bump(&mut self) {
        self.bump_tracker.assert_no_references();
        for bump in &mut self.bump {
            bump.reset();
        }
    }
}

/// This is useful for the ECS, so we can use Single<&mut Broadcast> instead of having to use a marker struct
#[derive(Component, Default)]
pub struct IoBuf {
    buffer: ThreadLocal<RefCell<AlignedVec>>,
    // system_on: ThreadLocal<Cell<u32>>,
    // broadcast_buffer: ThreadLocal<RefCell<BytesMut>>,
    temp_buffer: ThreadLocal<RefCell<BytesMut>>,
    idx: ThreadLocal<Cell<u16>>,
    // TODO: Consider replacing this with some sort of append-only vec
    packet_number: AtomicU32,
}

impl IoBuf {
    pub fn fetch_add_idx(&self) -> u16 {
        let cell = self.idx.get_or_default();
        let result = cell.get();
        cell.set(result + 1);
        result
    }
}

/// A broadcast builder
#[must_use]
pub struct Broadcast<'a, P> {
    packet: P,
    compose: &'a Compose,
    exclude: u64,
}

/// A unicast builder
#[must_use]
struct Unicast<'a, P> {
    packet: P,
    stream_id: ConnectionId,
    compose: &'a Compose,
    compress: bool,
}

impl<P> Unicast<'_, P>
where
    P: PacketBundle,
{
    fn send(self) -> anyhow::Result<()> {
        self.compose.io_buf.unicast_private(
            self.packet,
            self.stream_id,
            self.compose,
            self.compress,
        )
    }
}

impl<P> Broadcast<'_, P> {
    /// Send the packet to all players.
    pub fn send(self) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let bytes = self
            .compose
            .io_buf
            .encode_packet(self.packet, self.compose)?;

        self.compose.io_buf.broadcast_raw(&bytes, self.exclude);

        Ok(())
    }

    /// Exclude a certain player from the broadcast. This can only be called once.
    pub fn exclude(self, exclude: impl Into<Option<ConnectionId>>) -> Self {
        let exclude = exclude.into();
        let exclude = exclude.map(|id| id.stream_id).unwrap_or_default();
        Broadcast {
            packet: self.packet,
            compose: self.compose,
            exclude,
        }
    }
}

#[must_use]
#[expect(missing_docs)]
pub struct BroadcastLocal<'a, P> {
    packet: P,
    compose: &'a Compose,
    center: ChunkPosition,
    exclude: u64,
}

impl<P> BroadcastLocal<'_, P> {
    /// Send the packet
    pub fn send(self) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let bytes = self
            .compose
            .io_buf
            .encode_packet(self.packet, self.compose)?;

        self.compose
            .io_buf
            .broadcast_local_raw(&bytes, self.center, self.exclude);

        Ok(())
    }

    /// Exclude a certain player from the broadcast. This can only be called once.
    pub fn exclude(self, exclude: impl Into<Option<ConnectionId>>) -> Self {
        let exclude = exclude.into();
        let exclude = exclude.map(|id| id.stream_id).unwrap_or_default();
        BroadcastLocal {
            packet: self.packet,
            compose: self.compose,
            center: self.center,
            exclude,
        }
    }
}

impl IoBuf {
    /// Returns an iterator over the result of splitting the buffer into packets with [`BytesMut::split`].
    pub fn reset_and_split(&mut self) -> impl Iterator<Item = Bytes> + '_ {
        // reset idx
        for elem in &mut self.idx {
            elem.set(0);
        }

        *self.packet_number.get_mut() = 0;

        self.buffer.iter_mut().map(|x| x.borrow_mut()).map(|mut x| {
            let res = Bytes::copy_from_slice(x.as_slice());
            x.clear();
            res
        })
    }

    fn next_packet_number(&self) -> u32 {
        // Relaxed ordering is allowed here. If a system wanted a packet to be sent before another
        // packet, there should already be a happens-before relationship between the two packets
        // which would allow the second packet to receive a higher packet number.
        self.packet_number.fetch_add(1, Ordering::Relaxed)
    }

    fn encode_packet<P>(&self, packet: P, compose: &Compose) -> anyhow::Result<BytesMut>
    where
        P: PacketBundle,
    {
        let temp_buffer = self.temp_buffer.get_or_default();
        let temp_buffer = &mut *temp_buffer.borrow_mut();

        let compressor = compose.compressor();
        let mut compressor = compressor.borrow_mut();

        let scratch = compose.scratch();
        let mut scratch = scratch.borrow_mut();

        let result =
            compose
                .encoder()
                .append_packet(packet, temp_buffer, &mut *scratch, &mut compressor)?;

        Ok(result)
    }

    fn encode_packet_no_compression<P>(&self, packet: P) -> anyhow::Result<BytesMut>
    where
        P: PacketBundle,
    {
        let temp_buffer = self.temp_buffer.get_or_default();
        let temp_buffer = &mut *temp_buffer.borrow_mut();

        let result = append_packet_without_compression(packet, temp_buffer)?;

        Ok(result)
    }

    fn unicast_private<P>(
        &self,
        packet: P,
        id: ConnectionId,
        compose: &Compose,
        compress: bool,
    ) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let bytes = if compress {
            self.encode_packet(packet, compose)?
        } else {
            self.encode_packet_no_compression(packet)?
        };

        self.unicast_raw(&bytes, id);
        Ok(())
    }

    fn broadcast_local_raw(&self, data: &[u8], center: impl Into<ChunkPosition>, exclude: u64) {
        let center = center.into();

        let buffer = self.buffer.get_or_default();
        let buffer = &mut *buffer.borrow_mut();

        let order = self.next_packet_number();

        let to_send = hyperion_proto::BroadcastLocal {
            data,
            center,
            exclude,
            order,
        };

        let to_send = ServerToProxyMessage::BroadcastLocal(to_send);

        let len = buffer.len();
        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&to_send, &mut *buffer).unwrap();

        let new_len = buffer.len();
        let packet_len = u64::try_from(new_len - len - size_of::<u64>()).unwrap();
        buffer[len..(len + 8)].copy_from_slice(&packet_len.to_be_bytes());
    }

    pub(crate) fn broadcast_raw(&self, data: &[u8], exclude: u64) {
        let buffer = self.buffer.get_or_default();
        let buffer = &mut *buffer.borrow_mut();

        let order = self.next_packet_number();

        let to_send = hyperion_proto::BroadcastGlobal {
            data,
            // todo: Right now, we are using `to_vec`.
            // We want to probably allow encoding without allocation in the future.
            // Fortunately, `to_vec` will not require any allocation if the buffer is empty.
            exclude,
            order,
        };

        let to_send = ServerToProxyMessage::BroadcastGlobal(to_send);

        let len = buffer.len();
        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&to_send, &mut *buffer).unwrap();

        let new_len = buffer.len();
        let packet_len = u64::try_from(new_len - len - size_of::<u64>()).unwrap();
        buffer[len..(len + 8)].copy_from_slice(&packet_len.to_be_bytes());
    }

    pub(crate) fn unicast_raw(&self, data: &[u8], stream: ConnectionId) {
        let buffer = self.buffer.get_or_default();
        let buffer = &mut *buffer.borrow_mut();

        let order = self.next_packet_number();

        let to_send = hyperion_proto::Unicast {
            data,
            stream: stream.stream_id,
            order,
        };

        let to_send = ServerToProxyMessage::Unicast(to_send);

        let len = buffer.len();
        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&to_send, &mut *buffer).unwrap();

        let new_len = buffer.len();
        let packet_len = u64::try_from(new_len - len - size_of::<u64>()).unwrap();
        buffer[len..(len + 8)].copy_from_slice(&packet_len.to_be_bytes());
    }

    pub(crate) fn set_receive_broadcasts(&self, stream: ConnectionId) {
        let buffer = self.buffer.get_or_default();
        let buffer = &mut *buffer.borrow_mut();

        let to_send = hyperion_proto::SetReceiveBroadcasts {
            stream: stream.stream_id,
        };

        let to_send = ServerToProxyMessage::SetReceiveBroadcasts(to_send);

        let len = buffer.len();
        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&to_send, &mut *buffer).unwrap();

        let new_len = buffer.len();
        let packet_len = u64::try_from(new_len - len - size_of::<u64>()).unwrap();
        buffer[len..(len + 8)].copy_from_slice(&packet_len.to_be_bytes());
    }

    pub fn shutdown(&self, stream: ConnectionId) {
        let buffer = self.buffer.get_or_default();
        let buffer = &mut *buffer.borrow_mut();

        let to_send = hyperion_proto::Shutdown {
            stream: stream.stream_id,
        };

        let to_send = ServerToProxyMessage::Shutdown(to_send);

        let len = buffer.len();
        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&to_send, &mut *buffer).unwrap();

        let new_len = buffer.len();
        let packet_len = u64::try_from(new_len - len - size_of::<u64>()).unwrap();
        buffer[len..(len + 8)].copy_from_slice(&packet_len.to_be_bytes());
    }
}
