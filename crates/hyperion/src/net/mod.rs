//! All the networking related code.

use std::{cell::RefCell, fmt::Debug};

use byteorder::WriteBytesExt;
use bytes::{Bytes, BytesMut};
pub use decoder::PacketDecoder;
use flecs_ecs::prelude::*;
use glam::I16Vec2;
use hyperion_proxy_proto::{ChunkPosition, ServerToProxyMessage};
use hyperion_utils::EntityExt;
use libdeflater::CompressionLvl;
use rustc_hash::FxHashMap;
use thread_local::ThreadLocal;
use tracing::error;

use crate::{
    Global, PacketBundle, Scratch,
    net::{
        encoder::{PacketEncoder, append_packet_without_compression},
        intermediate::IntermediateServerToProxyMessage,
    },
    simulation::EgressComm,
};

pub mod agnostic;
pub mod decoder;
pub mod encoder;
pub mod intermediate;
pub mod protocol;
pub mod proxy;

/// The Minecraft protocol version this library currently targets.
pub const PROTOCOL_VERSION: i32 = hyperion_minecraft_proto::PROTOCOL_VERSION;

/// The maximum number of bytes that can be sent in a single packet.
pub const MAX_PACKET_SIZE: usize = valence_protocol::MAX_PACKET_SIZE as usize;

/// The stringified name of the Minecraft version this library currently
/// targets.
pub const MINECRAFT_VERSION: &str = hyperion_minecraft_proto::MINECRAFT_VERSION;

/// A unique identifier for a proxy to game server connection
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProxyId {
    /// The underlying unique identifier for the proxy connection.
    /// This value is guaranteed to be unique among all active connections.
    proxy_id: u64,
}

impl ProxyId {
    /// Creates a new proxy ID with the specified proxy identifier.
    ///
    /// This is an internal API used by the proxy management system.
    #[must_use]
    pub const fn new(proxy_id: u64) -> Self {
        Self { proxy_id }
    }

    /// Returns the underlying proxy identifier.
    ///
    /// This method is primarily used by internal networking code to interact
    /// with the proxy layer. Most application code should not need this.
    #[must_use]
    pub const fn inner(self) -> u64 {
        self.proxy_id
    }
}

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
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId {
    /// The underlying unique identifier for this connection.
    /// This value is guaranteed to be unique among all active connections.
    stream_id: u64,

    /// The proxy which this player connection is connected to
    proxy_id: ProxyId,
}

impl ConnectionId {
    /// Creates a new connection ID with the specified stream identifier.
    ///
    /// This is an internal API used by the connection management system.
    /// External code should obtain connection IDs through the appropriate
    /// connection handlers.
    #[must_use]
    pub const fn new(stream_id: u64, proxy_id: ProxyId) -> Self {
        Self {
            stream_id,
            proxy_id,
        }
    }

    /// Returns the proxy which this player connection is connected to.
    ///
    /// This method is primarily used by internal networking code.
    /// Most application code should not need this.
    #[must_use]
    pub const fn proxy_id(self) -> ProxyId {
        self.proxy_id
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
        Self::new(bytemuck::cast(entity.minecraft_id()))
    }
}

impl From<EntityView<'_>> for ChannelId {
    fn from(entity: EntityView<'_>) -> Self {
        Self::from(entity.id())
    }
}

/// A singleton that can be used to compose and encode packets.
#[derive(Component)]
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

    /// Send to every player, optionally skipping one.
    ///
    /// `exclude` is not a nicety: the refresh that re-sends a player to
    /// everyone tells the others to drop and re-add that player's entity, and
    /// the one client that must never be told to drop it is the player
    /// themselves.
    pub fn broadcast(&self, exclude: Option<ConnectionId>) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose.io_buf.broadcast_raw(&self.data, exclude);
        Ok(())
    }

    // todo: use builder pattern for excluding
    pub fn broadcast_local(&self, center: I16Vec2) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose
            .io_buf
            .broadcast_local_raw(&self.data, center, None);
        Ok(())
    }

    // todo: use builder pattern for excluding
    pub fn broadcast_channel(&self, channel: ChannelId) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose
            .io_buf
            .broadcast_channel_raw(&self.data, channel, None);

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
    /// Reaches the proxy as [`hyperion_proxy_proto::BroadcastGlobal`].
    pub const fn broadcast<P>(&self, packet: P) -> Broadcast<'_, P>
    where
        P: PacketBundle,
    {
        Broadcast {
            packet,
            compose: self,
            exclude: None,
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
    /// Reaches the proxy as [`hyperion_proxy_proto::BroadcastLocal`].
    pub const fn broadcast_local<P>(&self, packet: P, center: I16Vec2) -> BroadcastLocal<'_, P>
    where
        P: PacketBundle,
    {
        BroadcastLocal {
            packet,
            compose: self,
            exclude: None,
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
            exclude: None,
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
    egress_comms: FxHashMap<ProxyId, EgressComm>,
}

impl IoBuf {
    pub(crate) fn add_proxy(&mut self, proxy_id: ProxyId, egress_comm: EgressComm) {
        let already_exists = self.egress_comms.insert(proxy_id, egress_comm).is_some();

        if already_exists {
            error!("added multiple proxies with the same proxy id {proxy_id:?}");
        }
    }

    pub(crate) fn remove_proxy(&mut self, proxy_id: ProxyId) -> Option<EgressComm> {
        self.egress_comms.remove(&proxy_id)
    }
}

/// A broadcast builder
#[must_use]
pub struct Broadcast<'a, P> {
    packet: P,
    compose: &'a Compose,
    exclude: Option<ConnectionId>,
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
    exclude: Option<ConnectionId>,
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
    exclude: Option<ConnectionId>,
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

    pub(crate) fn encode_proxy_message(message: &ServerToProxyMessage<'_>) -> Bytes {
        let mut buffer = Vec::<u8>::new();

        buffer.write_u64::<byteorder::BigEndian>(0x00).unwrap();

        rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(message, &mut buffer).unwrap();

        let packet_len = u64::try_from(buffer.len() - size_of::<u64>()).unwrap();
        buffer[0..8].copy_from_slice(&packet_len.to_be_bytes());

        Bytes::from_owner(buffer)
    }

    pub(crate) fn add_proxy_message(&self, message: &IntermediateServerToProxyMessage<'_>) {
        if message.affected_by_proxy() {
            // Encode the message for each proxy before sending it
            for (&proxy_id, egress_comm) in &self.egress_comms {
                let Some(message) = message.transform_for_proxy(proxy_id) else {
                    continue;
                };

                egress_comm
                    .tx
                    .send(Self::encode_proxy_message(&message))
                    .unwrap();
            }
        } else {
            // Encode the message once and then send it to each proxy. This uses a placeholder
            // proxy id.
            let Some(message) = message.transform_for_proxy(ProxyId::new(0)) else {
                return;
            };

            let buffer = Self::encode_proxy_message(&message);
            for egress_comm in self.egress_comms.values() {
                egress_comm.tx.send(buffer.clone()).unwrap();
            }
        }
    }

    fn broadcast_local_raw(
        &self,
        data: &[u8],
        center: impl Into<ChunkPosition>,
        exclude: Option<ConnectionId>,
    ) {
        let center = center.into();

        self.add_proxy_message(&IntermediateServerToProxyMessage::BroadcastLocal(
            intermediate::BroadcastLocal {
                center,
                exclude,
                data,
            },
        ));
    }

    fn broadcast_channel_raw(
        &self,
        data: &[u8],
        channel: ChannelId,
        exclude: Option<ConnectionId>,
    ) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::BroadcastChannel(
            intermediate::BroadcastChannel {
                channel_id: channel.inner(),
                data,
                exclude,
            },
        ));
    }

    pub(crate) fn broadcast_raw(&self, data: &[u8], exclude: Option<ConnectionId>) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::BroadcastGlobal(
            intermediate::BroadcastGlobal { exclude, data },
        ));
    }

    pub(crate) fn unicast_raw(&self, data: &[u8], stream: ConnectionId) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::Unicast(
            intermediate::Unicast { stream, data },
        ));
    }

    pub(crate) fn set_receive_broadcasts(&self, stream: ConnectionId) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::SetReceiveBroadcasts(
            intermediate::SetReceiveBroadcasts { stream },
        ));
    }

    pub(crate) fn add_channel(&self, channel: ChannelId, unsubscribe_packets: &[u8]) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::AddChannel(
            intermediate::AddChannel {
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
        receiver: ProxyId,
    ) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::SubscribeChannelPackets(
            intermediate::SubscribeChannelPackets {
                channel_id: channel.inner(),
                exclude,
                receiver,
                data: packets,
            },
        ));
    }

    pub(crate) fn remove_channel(&self, channel: ChannelId) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::RemoveChannel(
            intermediate::RemoveChannel {
                channel_id: channel.inner(),
            },
        ));
    }

    pub fn shutdown(&self, stream: ConnectionId) {
        self.add_proxy_message(&IntermediateServerToProxyMessage::Shutdown(
            intermediate::Shutdown { stream },
        ));
    }
}

/// Helpers for driving this crate's egress from a downstream crate's tests.
///
/// Behind the off-by-default `test-util` feature, because everything here
/// exists to be called from a `#[cfg(test)]` block and nothing in a running
/// server should reach it.
///
/// # Why this is a feature and not four dependencies
///
/// Building a readable [`Compose`] from outside `hyperion` needs
/// [`IoBuf::add_proxy`] (crate-private, and a real API this should not widen),
/// `libdeflater::CompressionLvl`, `valence_protocol::CompressionThreshold` and
/// a `tokio` channel; reading a frame back needs `rkyv` and
/// `hyperion_proxy_proto`. That is six things a consumer would have to depend
/// on and keep in step to assert on one packet. Two functions behind a flag is
/// the smaller surface.
#[cfg(feature = "test-util")]
pub mod test_util {
    use bytes::Bytes;
    use hyperion_proxy_proto::ArchivedServerToProxyMessage;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::Compose;

    /// A [`Compose`] wired to one proxy, and that proxy's receiving end.
    ///
    /// Everything the game sends to this player lands on the returned channel
    /// instead of a socket, so a test can read what a client would have been
    /// told rather than what the game intended to tell them.
    #[must_use]
    pub fn compose_with_proxy() -> (Compose, UnboundedReceiver<Bytes>) {
        let (compose, zero, _one) = super::tests::two_proxies();
        (compose, zero)
    }

    /// The raw packet bytes of the next unicast frame, or `None` when the
    /// channel is empty or the next frame is not a unicast.
    ///
    /// The rkyv step lives here rather than in the caller so a consumer needs
    /// no `rkyv` or `hyperion_proxy_proto` dependency of its own.
    #[must_use]
    pub fn next_unicast(rx: &mut UnboundedReceiver<Bytes>) -> Option<Vec<u8>> {
        let bytes = rx.try_recv().ok()?;
        // `encode_proxy_message` writes an eight-byte big-endian length before
        // the rkyv body; see `tests::next_variant` for the alignment argument.
        let body = &bytes[size_of::<u64>()..];
        let message = unsafe { rkyv::access_unchecked::<ArchivedServerToProxyMessage<'_>>(body) };
        match message {
            ArchivedServerToProxyMessage::Unicast(unicast) => Some(unicast.data.to_vec()),
            _ => None,
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
pub(crate) mod tests {
    use std::sync::Arc;

    // These three reach only `next_variant` and the `#[test]` functions at the
    // bottom of this module, all of which are `cfg(test)`. Under `test-util`
    // alone they are unused imports and `-D warnings` rejects them.
    #[cfg(test)]
    use hyperion_proxy_proto::ArchivedServerToProxyMessage;

    #[cfg(test)]
    use super::{ChannelId, ConnectionId};
    // Named rather than a glob: `test-util` compiles this module outside
    // `cfg(test)`, where the workspace's pedantic `wildcard_imports` applies.
    use super::{Compose, CompressionLvl, IoBuf, ProxyId};
    use crate::{CompressionThreshold, Global, common::Shared, simulation::EgressComm};

    /// A [`Compose`] with two proxies registered, and the receiving end of each proxy's channel.
    ///
    /// This is the smallest thing that can tell the two multi-proxy failure modes apart: with one
    /// proxy every message goes to the only channel there is, so nothing that routes wrongly can
    /// be observed at all.
    pub fn two_proxies() -> (
        Compose,
        tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
        tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    ) {
        let (zero_tx, zero_rx) = tokio::sync::mpsc::unbounded_channel();
        let (one_tx, one_rx) = tokio::sync::mpsc::unbounded_channel();

        let mut io_buf = IoBuf::default();
        io_buf.add_proxy(ProxyId::new(0), EgressComm::from(zero_tx));
        io_buf.add_proxy(ProxyId::new(1), EgressComm::from(one_tx));

        let compression_level = CompressionLvl::new(4).unwrap();
        let compose = Compose::new(
            compression_level,
            Global::new(Arc::new(Shared {
                compression_threshold: CompressionThreshold(256),
                compression_level,
            })),
            io_buf,
        );

        (compose, zero_rx, one_rx)
    }

    /// Reads one framed message off a proxy's channel and tells you which variant it was.
    ///
    /// `cfg(test)` and not `test-util`, because the only callers are this crate's own
    /// tests. `test-util` compiles this module into the LIB target, where an item nothing
    /// outside `cfg(test)` calls is dead code -- and `checks.clippy` runs
    /// `--all-targets --all-features -- -D warnings`, so dead code there is a build
    /// failure rather than a warning. Widen this to the whole module the day something
    /// outside the crate needs it.
    #[cfg(test)]
    pub fn next_variant(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    ) -> Option<String> {
        let bytes = rx.try_recv().ok()?;
        // `encode_proxy_message` writes an eight-byte big-endian length before the rkyv body.
        let body = &bytes[size_of::<u64>()..];
        // rkyv reads the root through self-relative pointers at the end of the buffer, so the
        // body has to be aligned for the archived type. The eight-byte prefix preserves alignment
        // mod 8; what it relies on is the allocator handing `Bytes` a block that was aligned to
        // begin with, which is true of the system allocator but not promised by `Vec<u8>`'s
        // layout. Assert rather than discover it as a misread field.
        debug_assert_eq!(
            body.as_ptr() as usize % align_of::<u64>(),
            0,
            "the rkyv body must be aligned for the archived message"
        );
        let message = unsafe { rkyv::access_unchecked::<ArchivedServerToProxyMessage<'_>>(body) };
        Some(
            match message {
                ArchivedServerToProxyMessage::UpdatePlayerPositions(_) => "UpdatePlayerPositions",
                ArchivedServerToProxyMessage::AddChannel(_) => "AddChannel",
                ArchivedServerToProxyMessage::UpdateChannelPositions(_) => "UpdateChannelPositions",
                ArchivedServerToProxyMessage::RemoveChannel(_) => "RemoveChannel",
                ArchivedServerToProxyMessage::SubscribeChannelPackets(_) => {
                    "SubscribeChannelPackets"
                }
                ArchivedServerToProxyMessage::BroadcastGlobal(_) => "BroadcastGlobal",
                ArchivedServerToProxyMessage::BroadcastLocal(_) => "BroadcastLocal",
                ArchivedServerToProxyMessage::BroadcastChannel(_) => "BroadcastChannel",
                ArchivedServerToProxyMessage::Unicast(_) => "Unicast",
                ArchivedServerToProxyMessage::SetReceiveBroadcasts(_) => "SetReceiveBroadcasts",
                ArchivedServerToProxyMessage::Shutdown(_) => "Shutdown",
            }
            .to_owned(),
        )
    }

    /// `IoBuf::shutdown` is what every validation failure calls. What lands on the wire has to be
    /// a `Shutdown`, on the one proxy holding the connection, and nothing at all on the other.
    #[test]
    fn shutdown_reaches_one_proxy_as_a_shutdown() {
        let (compose, mut zero_rx, mut one_rx) = two_proxies();

        compose
            .io_buf()
            .shutdown(ConnectionId::new(1, ProxyId::new(0)));

        assert_eq!(
            next_variant(&mut zero_rx).as_deref(),
            Some("Shutdown"),
            "the proxy holding the connection must be told to close it"
        );
        assert!(
            next_variant(&mut one_rx).is_none(),
            "a proxy that does not hold the connection must hear nothing"
        );
    }

    /// The answer to a subscribe request belongs to the proxy that asked.
    #[test]
    fn a_subscribe_is_delivered_only_to_the_asking_proxy() {
        let (compose, mut zero_rx, mut one_rx) = two_proxies();

        compose.io_buf().send_subscribe_channel_packets(
            ChannelId::new(7),
            &[1, 2, 3],
            None,
            ProxyId::new(0),
        );

        assert_eq!(
            next_variant(&mut zero_rx).as_deref(),
            Some("SubscribeChannelPackets"),
            "the proxy that asked must get its answer"
        );
        assert!(
            next_variant(&mut one_rx).is_none(),
            "a proxy that never asked must not be told to subscribe anyone"
        );
    }
}
