//! All the networking related code.

use std::{
    cell::{Cell, RefCell},
    fmt::Debug,
};

use bevy::prelude::*;
use byteorder::WriteBytesExt;
use bytes::{Bytes, BytesMut};
pub use decoder::PacketDecoder;
use glam::I16Vec2;
use hyperion_proto::{ChunkPosition, ServerToProxyMessage};
use hyperion_utils::EntityExt;
use libdeflater::CompressionLvl;
use thread_local::ThreadLocal;

use crate::{
    Global, PacketBundle, Scratch,
    net::encoder::{PacketEncoder, append_packet_without_compression},
    simulation::EgressComm,
};

pub mod agnostic;
pub mod decoder;
pub mod encoder;
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

/// A component marking an entity as a packet channel.
#[derive(Component, Copy, Clone, Debug)]
pub struct Channel;

/// A unique identifier for a channel. The server is responsible for managing channel IDs.
#[derive(Component, Copy, Clone, Debug)]
pub struct ChannelId {
    /// The underlying unique identifier for this channel.
    channel_id: u32,
}

impl ChannelId {
    /// Creates a new channel ID with the specified stream identifier.
    #[must_use]
    pub const fn new(channel_id: u32) -> Self {
        Self { channel_id }
    }

    /// Returns the underlying channel identifier.
    ///
    /// This method is primarily used by internal networking code to interact
    /// with the proxy layer. Most application code should work with the
    /// `ChannelId` type directly rather than the raw ID.
    #[must_use]
    pub const fn inner(self) -> u32 {
        self.channel_id
    }
}

impl From<Entity> for ChannelId {
    fn from(entity: Entity) -> Self {
        Self::new(entity.id())
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

    // todo: use builder pattern for excluding
    pub fn broadcast_channel(&self, channel: ChannelId) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose
            .io_buf
            .broadcast_channel_raw(&self.data, channel, 0);

        Ok(())
    }
}

impl Compose {
    #[must_use]
    pub const fn new(compression_lvl: CompressionLvl, global: Global, io_buf: IoBuf) -> Self {
        Self {
            compression_lvl,
            compressor: ThreadLocal::new(),
            scratch: ThreadLocal::new(),
            global,
            io_buf,
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
    pub const fn broadcast<P>(&self, packet: P) -> Broadcast<'_, P>
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
    pub const fn broadcast_local<P>(&self, packet: P, center: I16Vec2) -> BroadcastLocal<'_, P>
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

    /// Broadcast a packet in a channel.
    pub const fn broadcast_channel<P>(
        &self,
        packet: P,
        channel: ChannelId,
    ) -> BroadcastChannel<'_, P>
    where
        P: PacketBundle,
    {
        BroadcastChannel {
            packet,
            compose: self,
            exclude: 0,
            channel,
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
}

/// This is useful for the ECS, so we can use Single<&mut Broadcast> instead of having to use a marker struct
#[derive(Component, Default)]
pub struct IoBuf {
    // system_on: ThreadLocal<Cell<u32>>,
    // broadcast_buffer: ThreadLocal<RefCell<BytesMut>>,
    temp_buffer: ThreadLocal<RefCell<BytesMut>>,
    idx: ThreadLocal<Cell<u16>>,
    egress_comms: Vec<EgressComm>,
}

impl IoBuf {
    pub fn fetch_add_idx(&self) -> u16 {
        let cell = self.idx.get_or_default();
        let result = cell.get();
        cell.set(result + 1);
        result
    }

    pub(crate) fn add_egress_comm(&mut self, egress_comm: EgressComm) {
        self.egress_comms.push(egress_comm);
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

#[must_use]
#[expect(missing_docs)]
pub struct BroadcastChannel<'a, P> {
    packet: P,
    compose: &'a Compose,
    exclude: u64,
    channel: ChannelId,
}

impl<P> BroadcastChannel<'_, P> {
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
            .broadcast_channel_raw(&bytes, self.channel, self.exclude);

        Ok(())
    }

    /// Exclude a certain player from the broadcast. This can only be called once.
    pub fn exclude(self, exclude: impl Into<Option<ConnectionId>>) -> Self {
        let exclude = exclude.into();
        let exclude = exclude.map(|id| id.stream_id).unwrap_or_default();
        Self { exclude, ..self }
    }
}

impl IoBuf {
    pub fn encode_packet<P>(&self, packet: P, compose: &Compose) -> anyhow::Result<BytesMut>
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

    pub fn encode_packet_no_compression<P>(&self, packet: P) -> anyhow::Result<BytesMut>
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

    pub(crate) fn add_proxy_message(&self, message: &ServerToProxyMessage<'_>) {
        let mut buffer = Vec::<u8>::new();

        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(message, &mut buffer).unwrap();

        let packet_len = u64::try_from(buffer.len() - size_of::<u64>()).unwrap();
        buffer[0..8].copy_from_slice(&packet_len.to_be_bytes());

        let buffer = Bytes::from_owner(buffer);

        for egress_comm in &self.egress_comms {
            egress_comm.tx.send(buffer.clone()).unwrap();
        }
    }

    fn broadcast_local_raw(&self, data: &[u8], center: impl Into<ChunkPosition>, exclude: u64) {
        let center = center.into();

        self.add_proxy_message(&ServerToProxyMessage::BroadcastLocal(
            hyperion_proto::BroadcastLocal {
                data,
                center,
                exclude,
            },
        ));
    }

    fn broadcast_channel_raw(&self, data: &[u8], channel: ChannelId, exclude: u64) {
        self.add_proxy_message(&ServerToProxyMessage::BroadcastChannel(
            hyperion_proto::BroadcastChannel {
                channel_id: channel.inner(),
                data,
                exclude,
            },
        ));
    }

    pub(crate) fn broadcast_raw(&self, data: &[u8], exclude: u64) {
        self.add_proxy_message(&ServerToProxyMessage::BroadcastGlobal(
            hyperion_proto::BroadcastGlobal {
                data,
                // todo: Right now, we are using `to_vec`.
                // We want to probably allow encoding without allocation in the future.
                // Fortunately, `to_vec` will not require any allocation if the buffer is empty.
                exclude,
            },
        ));
    }

    pub(crate) fn unicast_raw(&self, data: &[u8], stream: ConnectionId) {
        self.add_proxy_message(&ServerToProxyMessage::Unicast(hyperion_proto::Unicast {
            data,
            stream: stream.stream_id,
        }));
    }

    pub(crate) fn set_receive_broadcasts(&self, stream: ConnectionId) {
        self.add_proxy_message(&ServerToProxyMessage::SetReceiveBroadcasts(
            hyperion_proto::SetReceiveBroadcasts {
                stream: stream.stream_id,
            },
        ));
    }

    pub(crate) fn add_channel(&self, channel: ChannelId, unsubscribe_packets: &[u8]) {
        self.add_proxy_message(&ServerToProxyMessage::AddChannel(
            hyperion_proto::AddChannel {
                channel_id: channel.inner(),
                unsubscribe_packets,
            },
        ));
    }

    pub(crate) fn send_subscribe_channel_packets(
        &self,
        channel: ChannelId,
        packets: &[u8],
        exclude: Option<ConnectionId>,
    ) {
        self.add_proxy_message(&ServerToProxyMessage::SubscribeChannelPackets(
            hyperion_proto::SubscribeChannelPackets {
                channel_id: channel.inner(),
                exclude: exclude.map_or(0, |connection_id| connection_id.stream_id),
                data: packets,
            },
        ));
    }

    pub(crate) fn remove_channel(&self, channel: ChannelId) {
        self.add_proxy_message(&ServerToProxyMessage::RemoveChannel(
            hyperion_proto::RemoveChannel {
                channel_id: channel.inner(),
            },
        ));
    }

    pub fn shutdown(&self, stream: ConnectionId) {
        self.add_proxy_message(&ServerToProxyMessage::Shutdown(hyperion_proto::Shutdown {
            stream: stream.stream_id,
        }));
    }
}
