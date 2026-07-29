//! Decoding play packets.
//!
//! The four states before play are decoded by [`crate::net::protocol`], which
//! reads the same frames through the same [`PacketDecoder`]. They are separate
//! because a pre-play packet decides which state the *next* packet is read in,
//! so those systems read one packet per tick, while play has no such ordering
//! constraint and drains everything queued.

use std::cell::RefCell;

use flecs_ecs::prelude::*;
use thread_local::ThreadLocal;
use tracing::error;

use crate::{
    net::{Compose, ConnectionId, PacketDecoder, decoder::BorrowedPacketFrame},
    simulation::{
        ConfirmBlockSequences, EntitySize, PacketState, Pitch, Position, Yaw,
        animation::ActiveAnimation,
        blocks::Blocks,
        handlers::{PacketSwitchQuery, packet_switch},
        metadata::entity::Pose,
        packet::HandlerRegistry,
    },
    storage::Events,
};

/// One zlib decompressor per thread, shared by every system that reads frames.
///
/// A `libdeflater::Decompressor` owns a working buffer, so it is worth keeping
/// rather than constructing per packet, and it is not `Sync`.
#[derive(Default, Component)]
pub struct Decompressor(pub ThreadLocal<RefCell<libdeflater::Decompressor>>);

/// Read one frame, shutting the connection down if it does not decode.
pub(crate) fn try_next_frame(
    compose: &Compose,
    connection_id: ConnectionId,
    decoder: &PacketDecoder,
    decompressor: &mut libdeflater::Decompressor,
    receiver: &mut packet_channel::Receiver,
) -> Option<BorrowedPacketFrame> {
    let raw_packet = receiver.try_recv()?;
    match decoder.try_next_packet(decompressor, raw_packet) {
        Ok(packet) => Some(packet),
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

/// Decodes and dispatches play packets.
///
/// Single-threaded, because handlers spawn entities and flecs forbids that
/// inside a multithreaded system.
fn register_play(world: &World) {
    system!(
        "recv_data",
        world,
        &Compose,
        &Blocks,
        &HandlerRegistry,
        &Events,
        &hyperion_crafting::CraftingRegistry,
        &Decompressor,
        &ConnectionId,
        &PacketDecoder,
        &mut packet_channel::Receiver,
        ?&mut Position,
        &mut Yaw,
        &mut Pitch,
        &mut EntitySize,
        ?&mut Pose,
        &mut ConfirmBlockSequences,
        &mut hyperion_inventory::PlayerInventory,
        &mut ActiveAnimation,
    )
    .with_enum(PacketState::Play)
    .kind(id::<flecs::pipeline::OnUpdate>())
    .each_iter(
        |it,
         row,
         (
            compose,
            blocks,
            handler_registry,
            events,
            crafting_registry,
            decompressor,
            connection_id,
            decoder,
            receiver,
            position,
            yaw,
            pitch,
            size,
            pose,
            confirm_block_sequences,
            inventory,
            animation,
        )| {
            let world = it.world();
            let view = it.entity(row);
            let id = view.id();
            let connection_id = *connection_id;
            let mut decompressor = decompressor.0.get_or_default().borrow_mut();

            // Transitioning to play is just a way to make sure that the player is officially in
            // play before we start sending them play packets, so a player without a position or
            // pose is not ready to be simulated yet.
            let Some((position, pose)) = position.zip(pose) else {
                return;
            };

            let mut query = PacketSwitchQuery {
                id,
                handler_registry,
                view,
                compose,
                io_ref: connection_id,
                position,
                yaw,
                pitch,
                size,
                world: &world,
                blocks,
                pose,
                events,
                confirm_block_sequences,
                inventory,
                animation,
                crafting_registry,
            };

            while let Some(frame) =
                try_next_frame(compose, connection_id, decoder, &mut decompressor, receiver)
            {
                let frame_id = frame.id;
                if let Err(e) = packet_switch(frame, &mut query) {
                    error!("error while processing packet (id: {frame_id}): {e}");
                }
            }
        },
    );
}

/// Registration module for the play-decode path: the per-connection frame
/// [`Receiver`](packet_channel::Receiver) and the shared [`Decompressor`].
///
/// Registration only, per the flecs convention in the root `CLAUDE.md`. The
/// systems that read them live in [`DecodeModule`], which imports this.
#[derive(Component)]
pub struct DecodeComponentsModule;

impl Module for DecodeComponentsModule {
    fn module(world: &World) {
        world.component::<packet_channel::Receiver>();

        // Registered as a singleton before it is set. A bare `set` stores the
        // value without registering the type, and that is the dev-only
        // ECS_INVALID_OPERATION abort of ENG-11000.
        world
            .component::<Decompressor>()
            .add_trait::<flecs::Singleton>();
        world.set(Decompressor::default());
    }
}

/// Behavior module for the play-decode path: the `recv_data` system that drains
/// every queued frame and dispatches it through [`packet_switch`].
#[derive(Component)]
pub struct DecodeModule;

impl Module for DecodeModule {
    fn module(world: &World) {
        world.import::<DecodeComponentsModule>();
        // `recv_data` reads `Position`, `Yaw`, `Pitch` and `EntitySize` off the
        // player it is decoding for, so the module registering the kinematics
        // base is imported rather than assumed.
        world.import::<crate::simulation::KinematicsComponentsModule>();

        register_play(world);
    }
}
