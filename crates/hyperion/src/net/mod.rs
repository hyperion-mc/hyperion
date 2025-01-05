//! All the networking related code.

use std::{
    cell::{Cell, RefCell},
    fmt::Debug,
};

use bumpalo::Bump;
use byteorder::WriteBytesExt;
use bytes::{Bytes, BytesMut};
pub use decoder::PacketDecoder;
use derive_more::Deref;
use flecs_ecs::{
    core::{EntityView, World, WorldProvider},
    macros::Component,
};
use glam::I16Vec2;
use hyperion_proto::{ChunkPosition, ServerToProxyMessage};
use libdeflater::CompressionLvl;
use rkyv::util::AlignedVec;
use system_order::SystemOrder;

use crate::{
    Global, PacketBundle, Scratch, Scratches,
    net::encoder::{PacketEncoder, append_packet_without_compression},
    storage::ThreadLocal,
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

/// Thread-local [`libdeflater::Compressor`] for encoding packets.
#[derive(Component, Deref)]
pub struct Compressors {
    compressors: ThreadLocal<RefCell<libdeflater::Compressor>>,
}

impl Compressors {
    #[must_use]
    pub(crate) fn new(level: CompressionLvl) -> Self {
        Self {
            compressors: ThreadLocal::new_with(|_| {
                RefCell::new(libdeflater::Compressor::new(level))
            }),
        }
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
#[derive(Component)]
pub struct Compose {
    compressor: Compressors,
    scratch: Scratches,
    global: Global,
    io_buf: IoBuf,
    pub bump: ThreadLocal<Bump>,
}

#[must_use]
pub struct DataBundle<'a, 'b> {
    compose: &'a Compose,
    system: EntityView<'b>,
    data: BytesMut,
}

impl<'a, 'b> DataBundle<'a, 'b> {
    pub fn new(compose: &'a Compose, system: EntityView<'b>) -> Self {
        Self {
            compose,
            system,
            data: BytesMut::new(),
        }
    }

    pub fn add_packet(&mut self, pkt: impl PacketBundle) -> anyhow::Result<()> {
        let world = self.system.world();
        let data = self
            .compose
            .io_buf
            .encode_packet(pkt, self.compose, &world)?;
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

        self.compose
            .io_buf
            .unicast_raw(&self.data, stream, self.system);
        Ok(())
    }

    // todo: use builder pattern for excluding
    pub fn broadcast_local(&self, center: I16Vec2) -> anyhow::Result<()> {
        if self.data.is_empty() {
            return Ok(());
        }

        self.compose
            .io_buf
            .broadcast_local_raw(&self.data, center, 0, self.system);
        Ok(())
    }
}

impl Compose {
    #[must_use]
    pub fn new(compressor: Compressors, scratch: Scratches, global: Global, io_buf: IoBuf) -> Self {
        Self {
            compressor,
            scratch,
            global,
            io_buf,
            bump: ThreadLocal::new_defaults(),
        }
    }

    #[must_use]
    #[expect(missing_docs)]
    pub const fn global(&self) -> &Global {
        &self.global
    }

    #[expect(missing_docs)]
    pub fn global_mut(&mut self) -> &mut Global {
        &mut self.global
    }

    /// Broadcast globally to all players
    ///
    /// See <https://github.com/andrewgazelka/hyperion-proto/blob/main/src/server_to_proxy.proto#L17-L22>
    pub const fn broadcast<'a, 'b, P>(
        &'a self,
        packet: P,
        system: EntityView<'b>,
    ) -> Broadcast<'a, 'b, P>
    where
        P: PacketBundle,
    {
        Broadcast {
            packet,
            compose: self,
            exclude: 0,
            system,
        }
    }

    #[must_use]
    #[expect(missing_docs)]
    pub const fn io_buf(&self) -> &IoBuf {
        &self.io_buf
    }

    #[expect(missing_docs)]
    pub fn io_buf_mut(&mut self) -> &mut IoBuf {
        &mut self.io_buf
    }

    /// Broadcast a packet within a certain region.
    ///
    /// See <https://github.com/andrewgazelka/hyperion-proto/blob/main/src/server_to_proxy.proto#L17-L22>
    pub const fn broadcast_local<'a, 'b, P>(
        &'a self,
        packet: P,
        center: I16Vec2,
        system: EntityView<'b>,
    ) -> BroadcastLocal<'a, 'b, P>
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
            system,
        }
    }

    /// Send a packet to a single player.
    pub fn unicast<P>(
        &self,
        packet: P,
        stream_id: ConnectionId,
        system: EntityView<'_>,
    ) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        Unicast {
            packet,
            stream_id,
            compose: self,
            system,

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
        system: EntityView<'_>,
    ) -> anyhow::Result<()>
    where
        P: valence_protocol::Packet + valence_protocol::Encode,
    {
        Unicast {
            packet,
            stream_id,
            compose: self,
            compress: false,
            system,
        }
        .send()
    }

    #[must_use]
    pub(crate) fn encoder(&self) -> PacketEncoder {
        let threshold = self.global.shared.compression_threshold;
        PacketEncoder::new(threshold)
    }

    /// Obtain a thread-local scratch buffer.
    #[must_use]
    pub fn scratch(&self, world: &World) -> &RefCell<Scratch> {
        self.scratch.get(world)
    }

    /// Obtain a thread-local [`libdeflater::Compressor`]
    #[must_use]
    pub fn compressor(&self, world: &World) -> &RefCell<libdeflater::Compressor> {
        self.compressor.get(world)
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
}

impl IoBuf {
    pub fn fetch_add_idx(&self, world: &World) -> u16 {
        let cell = self.idx.get(world);
        let result = cell.get();
        cell.set(result + 1);
        result
    }

    pub fn order_id(&self, system_order: SystemOrder, world: &World) -> u32 {
        u32::from(system_order.value()) << 16 | u32::from(self.fetch_add_idx(world))
    }
}

/// A broadcast builder
#[must_use]
pub struct Broadcast<'a, 'b, P> {
    packet: P,
    compose: &'a Compose,
    exclude: u64,
    system: EntityView<'b>,
}

/// A unicast builder
#[must_use]
struct Unicast<'a, 'b, P> {
    packet: P,
    stream_id: ConnectionId,
    compose: &'a Compose,
    compress: bool,
    system: EntityView<'b>,
}

impl<P> Unicast<'_, '_, P>
where
    P: PacketBundle,
{
    fn send(self) -> anyhow::Result<()> {
        self.compose.io_buf.unicast_private(
            self.packet,
            self.stream_id,
            self.compose,
            self.compress,
            self.system,
        )
    }
}

impl<P> Broadcast<'_, '_, P> {
    /// Send the packet to all players.
    pub fn send(self) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let world = self.system.world();

        let bytes = self
            .compose
            .io_buf
            .encode_packet(self.packet, self.compose, &world)?;

        self.compose
            .io_buf
            .broadcast_raw(&bytes, self.exclude, self.system);

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
            system: self.system,
        }
    }
}

#[must_use]
#[expect(missing_docs)]
pub struct BroadcastLocal<'a, 'b, P> {
    packet: P,
    compose: &'a Compose,
    center: ChunkPosition,
    exclude: u64,
    system: EntityView<'b>,
}

impl<P> BroadcastLocal<'_, '_, P> {
    /// Send the packet
    pub fn send(self) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let world = self.system.world();

        let bytes = self
            .compose
            .io_buf
            .encode_packet(self.packet, self.compose, &world)?;

        self.compose
            .io_buf
            .broadcast_local_raw(&bytes, self.center, self.exclude, self.system);

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
            system: self.system,
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

        self.buffer.iter_mut().map(|x| x.borrow_mut()).map(|mut x| {
            let res = Bytes::copy_from_slice(x.as_slice());
            x.clear();
            res
        })
    }

    fn encode_packet<P>(
        &self,
        packet: P,
        compose: &Compose,
        world: &World,
    ) -> anyhow::Result<BytesMut>
    where
        P: PacketBundle,
    {
        let temp_buffer = self.temp_buffer.get(world);
        let temp_buffer = &mut *temp_buffer.borrow_mut();

        let compressor = compose.compressor(world);
        let mut compressor = compressor.borrow_mut();

        let scratch = compose.scratch.get(world);
        let mut scratch = scratch.borrow_mut();

        let result =
            compose
                .encoder()
                .append_packet(packet, temp_buffer, &mut *scratch, &mut compressor)?;

        Ok(result)
    }

    fn encode_packet_no_compression<P>(&self, packet: P, world: &World) -> anyhow::Result<BytesMut>
    where
        P: PacketBundle,
    {
        let temp_buffer = self.temp_buffer.get(world);
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
        system: EntityView<'_>,
    ) -> anyhow::Result<()>
    where
        P: PacketBundle,
    {
        let world = system.world();

        let bytes = if compress {
            self.encode_packet(packet, compose, &world)?
        } else {
            self.encode_packet_no_compression(packet, &world)?
        };

        self.unicast_raw(&bytes, id, system);
        Ok(())
    }

    fn broadcast_local_raw(
        &self,
        data: &[u8],
        center: impl Into<ChunkPosition>,
        exclude: u64,
        system: EntityView<'_>,
    ) {
        let center = center.into();
        let world = system.world();
        let system_order = SystemOrder::of(system);

        let buffer = self.buffer.get(&world);
        let buffer = &mut *buffer.borrow_mut();

        let order = u32::from(system_order.value()) << 16;

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

    pub(crate) fn broadcast_raw(&self, data: &[u8], exclude: u64, system: EntityView<'_>) {
        let world = system.world();
        let buffer = self.buffer.get(&world);
        let buffer = &mut *buffer.borrow_mut();

        let system_order = SystemOrder::of(system);

        let order = u32::from(system_order.value()) << 16;

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

    pub(crate) fn unicast_raw(&self, data: &[u8], stream: ConnectionId, system: EntityView<'_>) {
        let world = system.world();
        let system_order = SystemOrder::of(system);

        let buffer = self.buffer.get(&world);
        let buffer = &mut *buffer.borrow_mut();

        let order = self.order_id(system_order, &world);

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

    pub(crate) fn set_receive_broadcasts(&self, stream: ConnectionId, world: &World) {
        let buffer = self.buffer.get(world);
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
}
