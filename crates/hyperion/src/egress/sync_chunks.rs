use std::cmp::Ordering;

use bevy::prelude::*;
use derive_more::derive::{Deref, DerefMut};
use glam::I16Vec2;
use itertools::Itertools;
use tracing::error;
use valence_protocol::{
    packets::play::{self}, ChunkPos,
    VarInt,
};

use crate::{
    config::Config,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        blocks::{Blocks, GetChunk}, ChunkPosition, PacketState, PacketStatePlay,
        Position,
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
        world.add_systems(Update, (generate_chunk_changes, send_full_loaded_chunks));
    }
}

fn generate_chunk_changes(
    compose: Res<'_, Compose>,
    mut query: Query<
        '_,
        '_,
        (&ConnectionId, &mut ChunkSendQueue, &Position),
        With<PacketStatePlay>,
    >,
) {
    query
        .par_iter_mut()
        .for_each(|(stream_id, mut chunk_changes, pose)| {
            let last_sent_chunk = last_sent.position;

            let current_chunk = pose.to_chunk();

            if last_sent_chunk == current_chunk {
                return;
            }

            // center chunk
            let center_chunk = play::ChunkRenderDistanceCenterS2c {
                chunk_x: VarInt(i32::from(current_chunk.x)),
                chunk_z: VarInt(i32::from(current_chunk.y)),
            };

            if let Err(e) = compose.unicast(&center_chunk, *stream_id, system) {
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

            let mut bundle = DataBundle::new(compose, system);

            for chunk in removed_chunks {
                let pos = ChunkPos::new(i32::from(chunk.x), i32::from(chunk.y));
                let unload_chunk = play::UnloadChunkS2c { pos };

                bundle.add_packet(&unload_chunk).unwrap();
            }

            bundle.unicast(stream_id).unwrap();

            let added_chunks = current_range_x
                .cartesian_product(current_range_z)
                .filter(|(x, y)| !last_sent_range_x.contains(x) || !last_sent_range_z.contains(y))
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
        });
}

fn send_full_loaded_chunks(
    compose: Res<'_, Compose>,
    blocks: Res<'_, Blocks>,
    mut query: Query<'_, '_, &ConnectionId, With<PacketStatePlay>>,
) {
    const MAX_CHUNKS_PER_TICK: usize = 16;

    query.par_iter_mut().for_each(|stream_id| {
        let last = None;

        let mut iter_count = 0;

        let mut bundle = DataBundle::new(&compose, system);

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

            if iter_count >= MAX_CHUNKS_PER_TICK {
                break;
            }

            match blocks.get_cached_or_load(elem) {
                GetChunk::Loaded(chunk) => {
                    bundle.add_raw(&chunk.base_packet_bytes);

                    for packet in chunk.original_delta_packets() {
                        if let Err(e) = bundle.add_packet(packet) {
                            error!("failed to send chunk delta packet: {e}");
                            return;
                        }
                    }

                    iter_count += 1;
                    #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
                    queue.changes.swap_remove(idx as usize);
                }
                GetChunk::Loading => {}
            }

            idx -= 1;
        }

        bundle.unicast(stream_id).unwrap();
    });
}
