//! Serving Minecraft 26.2 (protocol 776) with `hyperion-minecraft-proto`.
//!
//! Everything under here is behind the `proto-776` feature. The 763 path in
//! [`crate::ingress`] and [`crate::egress::player_join`] is unchanged and still
//! the default; this module replaces exactly the pre-play state machine and the
//! packets that put a player in the world, because those are the parts whose
//! wire format 776 changed beyond porting.
//!
//! The transport does not change with the protocol. hyperion frames packets
//! itself -- a `VarInt` length, then an optionally zlib-compressed body with a
//! `VarInt` id -- and the proxy forwards bytes without reading them, so
//! [`crate::net::encoder`], [`crate::net::decoder`] and `packet_channel` are
//! shared between the two protocols rather than duplicated.

use std::{cell::RefCell, io::Write};

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{Decode, Encode, Reader, Writer, types::KnownPack};
use itertools::Either;

use crate::{
    PacketBundle,
    net::{Compose, ConnectionId, decoder::BorrowedPacketFrame},
};

pub mod join;
pub mod pre_play;
pub mod registries;

/// The data packs this server would send registry contents for.
///
/// `BuiltInPackSource.CORE_PACK_INFO` is `KnownPack.vanilla("core")`, whose
/// version is the game version string. A client that reports the same pack
/// already has every registry element this server would send, so
/// [`registries`] can be names alone.
#[must_use]
pub fn known_packs() -> Vec<KnownPack<'static>> {
    vec![KnownPack {
        namespace: hyperion_minecraft_proto::packets::configuration::VANILLA_PACK_NAMESPACE,
        id: "core",
        version: crate::net::MINECRAFT_VERSION,
    }]
}

/// A clientbound 26.2 packet, paired with the id for the state it is sent in.
///
/// The proto crate deliberately keeps ids out of the packet structs, because
/// the same body has different ids in different states -- `keep_alive` and
/// `custom_payload` are `net.minecraft.network.protocol.common` classes shared
/// between configuration and play. Pairing them here is what lets one type
/// satisfy [`PacketBundle`] without the proto crate having to know which state
/// a caller is in.
pub struct Clientbound<'a, P> {
    id: i32,
    body: &'a P,
}

impl<'a, P: Encode> Clientbound<'a, P> {
    /// Pair a packet body with its on-wire id.
    pub const fn new(id: i32, body: &'a P) -> Self {
        Self { id, body }
    }
}

thread_local! {
    /// One encode buffer per thread, reused across packets.
    ///
    /// The play state sends a packet per entity per tick to every viewer, so a
    /// `Writer` that took a fresh `Vec` each time would put an allocation on
    /// the broadcast path. The buffer is only live for the body of
    /// `encode_including_ids`, which does not call back into itself, so one
    /// slot per thread is enough and a borrow conflict is not reachable.
    static ENCODE_BUFFER: RefCell<Writer> = RefCell::new(Writer::new());
}

impl<P: Encode> PacketBundle for Clientbound<'_, P> {
    fn encode_including_ids(self, mut w: impl Write) -> anyhow::Result<()> {
        ENCODE_BUFFER.with_borrow_mut(|writer| {
            writer.clear();
            writer.var_int(self.id);
            self.body.encode(writer)?;
            w.write_all(writer.as_slice())?;
            Ok(())
        })
    }
}

/// Send one packet to one connection, compressing it if compression is on.
///
/// # Errors
/// Returns an error when the packet does not encode, which for a packet this
/// server built itself means a protocol limit was exceeded.
pub fn send<P: Encode>(
    compose: &Compose,
    connection: ConnectionId,
    id: i32,
    body: &P,
) -> anyhow::Result<()> {
    compose.unicast(Clientbound::new(id, body), connection)
}

/// Send one packet to one connection without compressing it.
///
/// Only correct before `login_compression` has been sent: after that the client
/// expects every frame to carry a data-length prefix.
///
/// # Errors
/// See [`send`].
pub fn send_uncompressed<P: Encode>(
    compose: &Compose,
    connection: ConnectionId,
    id: i32,
    body: &P,
) -> anyhow::Result<()> {
    let bytes = compose
        .io_buf()
        .encode_packet_no_compression(Clientbound::new(id, body))?;
    compose.io_buf().unicast_raw(&bytes, connection);
    Ok(())
}

/// The bytes of a decoded frame, after the length prefix, compression and id
/// have been stripped.
///
/// A frame arrives as either an owned decompressed buffer or a borrow into the
/// channel's ring, and the proto crate reads from a plain slice, so this is the
/// one place the two representations are collapsed.
#[must_use]
pub fn frame_body(frame: &BorrowedPacketFrame) -> &[u8] {
    match &frame.body {
        Either::Left(bytes) => bytes,
        Either::Right(packet) => packet,
    }
}

/// Decode a packet body, failing if it does not consume the whole frame.
///
/// The trailing-bytes check is what turns a layout that is wrong in a later
/// field into an error here rather than a value that is silently off.
///
/// # Errors
/// Returns an error on a malformed body or on bytes left over.
pub fn decode_body<'a, T: Decode<'a>>(body: &'a [u8]) -> hyperion_minecraft_proto::Result<T> {
    let mut reader = Reader::new(body);
    let value = T::decode(&mut reader)?;
    reader.finish()?;
    Ok(value)
}

/// Registers every system that serves protocol 776.
#[derive(Component)]
pub struct ProtocolModule;

impl Module for ProtocolModule {
    fn module(world: &World) {
        world.import::<pre_play::PrePlayModule>();
        world.import::<join::JoinModule>();
    }
}
