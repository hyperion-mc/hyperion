//! Streaming terrain to a player as they move.
//!
//! Two systems: one turns a change of chunk into a queue of columns to send
//! and a set of columns to forget, and the other drains that queue at a fixed
//! rate so that a player joining does not get 400 columns in one tick.

use std::cmp::Ordering;

use derive_more::derive::{Deref, DerefMut};
use flecs_ecs::prelude::*;
use glam::I16Vec2;
use hyperion_minecraft_proto::{
    ChunkPos,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::clientbound::{
        ChunkBatchFinished, ChunkBatchStart, ForgetLevelChunk, SetChunkCacheCenter,
    },
};
use itertools::Itertools;
use tracing::error;

use crate::{
    config::Config,
    net::{
        Compose, ConnectionId, DataBundle,
        protocol::{Clientbound, send},
    },
    simulation::{
        ChunkPosition, PacketState, Position,
        blocks::{Blocks, GetChunk},
    },
};

#[derive(Component, Deref, DerefMut, Default)]
pub struct ChunkSendQueue {
    changes: Vec<I16Vec2>,
}

#[derive(Component)]
pub struct SyncChunksModule;

impl Module for SyncChunksModule {
    fn module(world: &World) {
        world.component::<ChunkSendQueue>();

        let radius = world.get::<&Config>(|config| config.view_distance);
        let liberal_radius = radius + 2;

        system!(
            "generate_chunk_changes",
            world,
            &Compose,
            &mut ChunkPosition,
            &Position,
            &ConnectionId,
            &mut ChunkSendQueue,
        )
        .with_enum(PacketState::Play)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(
            move |(compose, last_sent, pose, &stream_id, chunk_changes)| {
                let last_sent_chunk = last_sent.position;

                let current_chunk = pose.to_chunk();

                if last_sent_chunk == current_chunk {
                    return;
                }

                // The client discards any chunk outside its cache centre, so
                // this has to move before the columns around it are sent.
                let center_chunk = SetChunkCacheCenter {
                    x: i32::from(current_chunk.x),
                    z: i32::from(current_chunk.y),
                };

                if let Err(e) = send(
                    compose,
                    stream_id,
                    PacketId::SetChunkCacheCenter.to_raw(),
                    &center_chunk,
                ) {
                    error!(
                        "failed to send chunk render distance center packet: {e}. Chunk location: \
                         {current_chunk:?}"
                    );
                    return;
                }

                last_sent.position = current_chunk;

                let last_sent_range_x = (last_sent_chunk.x - radius)..(last_sent_chunk.x + radius);
                let last_sent_range_z = (last_sent_chunk.y - radius)..(last_sent_chunk.y + radius);

                let current_range_x = (current_chunk.x - radius)..(current_chunk.x + radius);
                let current_range_z = (current_chunk.y - radius)..(current_chunk.y + radius);

                let current_range_liberal_x =
                    (current_chunk.x - liberal_radius)..(current_chunk.x + liberal_radius);
                let current_range_liberal_z =
                    (current_chunk.y - liberal_radius)..(current_chunk.y + liberal_radius);

                chunk_changes.retain(|elem| {
                    current_range_liberal_x.contains(&elem.x)
                        && current_range_liberal_z.contains(&elem.y)
                });

                let removed_chunks = last_sent_range_x
                    .clone()
                    .cartesian_product(last_sent_range_z.clone())
                    .filter(|(x, y)| !current_range_x.contains(x) || !current_range_z.contains(y))
                    .map(|(x, y)| I16Vec2::new(x, y));

                let mut bundle = DataBundle::new(compose);

                for chunk in removed_chunks {
                    let forget =
                        ForgetLevelChunk(ChunkPos::new(i32::from(chunk.x), i32::from(chunk.y)));

                    bundle
                        .add_packet(Clientbound::new(
                            PacketId::ForgetLevelChunk.to_raw(),
                            &forget,
                        ))
                        .unwrap();
                }

                bundle.unicast(stream_id).unwrap();

                let added_chunks = current_range_x
                    .cartesian_product(current_range_z)
                    .filter(|(x, y)| {
                        !last_sent_range_x.contains(x) || !last_sent_range_z.contains(y)
                    })
                    .map(|(x, y)| I16Vec2::new(x, y));

                let mut num_chunks_added = 0;

                // drain all chunks not in current_{x,z} range

                for chunk in added_chunks {
                    chunk_changes.push(chunk);
                    num_chunks_added += 1;
                }

                if num_chunks_added > 0 {
                    // remove further than radius

                    // commented out because it can break things
                    // todo: re-add but have better check so we don't prune things and then never
                    // send them
                    // chunk_changes.retain(|elem| {
                    //     let elem = elem.distance_squared(current_chunk);
                    //     elem <= r2_very_liberal
                    // });

                    chunk_changes.sort_unstable_by(|a, b| {
                        let r1 = a.distance_squared(current_chunk);
                        let r2 = b.distance_squared(current_chunk);

                        // reverse because we want to get the closest chunks first and we are poping from the end
                        match r1.cmp(&r2).reverse() {
                            Ordering::Less => Ordering::Less,
                            Ordering::Greater => Ordering::Greater,

                            // so we can dedup properly (without same element could be scattered around)
                            Ordering::Equal => a.to_array().cmp(&b.to_array()),
                        }
                    });
                    chunk_changes.dedup();
                }
            },
        );

        system!(
            "send_full_loaded_chunks",
            world,
            &Blocks,
            &Compose,
            &ConnectionId,
            &mut ChunkSendQueue
        )
        .with_enum(PacketState::Play)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(move |(chunks, compose, &stream_id, queue)| {
            const MAX_CHUNKS_PER_TICK: usize = 16;

            let last = None;

            let mut ready = Vec::with_capacity(MAX_CHUNKS_PER_TICK);

            #[expect(
                clippy::cast_possible_wrap,
                reason = "realistically queue.changes.len() will never be large enough to wrap"
            )]
            let mut idx = (queue.changes.len() as isize) - 1;

            while idx >= 0 {
                #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
                let Some(elem) = queue.changes.get(idx as usize).copied() else {
                    // should never happen but we do not want to panic if wrong
                    // logic/assumptions are made
                    error!("failed to get element from queue.changes");
                    continue;
                };

                // de-duplicate. todo: there are cases where duplicate will not be removed properly
                // since sort is unstable
                if last == Some(elem) {
                    #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
                    queue.changes.swap_remove(idx as usize);
                    idx -= 1;
                    continue;
                }

                if ready.len() >= MAX_CHUNKS_PER_TICK {
                    break;
                }

                match chunks.get_cached_or_load(elem) {
                    GetChunk::Loaded(chunk) => {
                        ready.push(chunk);
                        #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
                        queue.changes.swap_remove(idx as usize);
                    }
                    GetChunk::Loading => {}
                }

                idx -= 1;
            }

            // The batch is what tells the client how fast this server can feed
            // it terrain: it times the run and answers with a chunks-per-tick
            // figure. Sending nothing at all is a batch of zero columns, which
            // would tell it this server is infinitely slow, so a tick with no
            // chunk ready sends no batch either.
            if ready.is_empty() {
                return;
            }

            let mut bundle = DataBundle::new(compose);

            if let Err(e) = bundle.add_packet(Clientbound::new(
                PacketId::ChunkBatchStart.to_raw(),
                &ChunkBatchStart,
            )) {
                error!("failed to start a chunk batch: {e}");
                return;
            }

            let batch_size = ready.len();
            for chunk in ready {
                bundle.add_raw(&chunk.base_packet_bytes);

                for packet in chunk.original_delta_packets() {
                    if let Err(e) = bundle.add_packet(packet) {
                        error!("failed to send chunk delta packet: {e}");
                        return;
                    }
                }
            }

            let finished = ChunkBatchFinished(i32::try_from(batch_size).unwrap_or(i32::MAX));
            if let Err(e) = bundle.add_packet(Clientbound::new(
                PacketId::ChunkBatchFinished.to_raw(),
                &finished,
            )) {
                error!("failed to finish a chunk batch: {e}");
                return;
            }

            bundle.unicast(stream_id).unwrap();
        });
    }
}
