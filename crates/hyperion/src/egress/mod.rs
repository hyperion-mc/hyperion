use bevy::prelude::*;
use byteorder::WriteBytesExt;
use hyperion_proto::{Flush, ServerToProxyMessage, UpdatePlayerChunkPositions};
use rkyv::util::AlignedVec;
use tracing::error;
use valence_protocol::{VarInt, packets::play::PlayerActionResponseS2c};

use crate::{
    Blocks,
    net::{Compose, ConnectionId},
    simulation::{ChunkPosition, EgressComm},
};
pub mod metadata;
pub mod player_join;
mod stats;
pub mod sync_chunks;
mod sync_entity_state;

use player_join::PlayerJoinPlugin;
use stats::StatsPlugin;
use sync_chunks::SyncChunksPlugin;
use sync_entity_state::EntityStateSyncPlugin;

#[derive(Resource)]
struct EncodedFlush(bytes::Bytes);

fn send_egress(
    mut compose: ResMut<'_, Compose>,
    egress: Res<'_, EgressComm>,
    flush: Res<'_, EncodedFlush>,
) {
    let io = compose.io_buf_mut();
    for bytes in io.reset_and_split() {
        if bytes.is_empty() {
            continue;
        }
        if let Err(e) = egress.send(bytes) {
            error!("failed to send egress: {e}");
        }
    }

    if let Err(e) = egress.send(flush.0.clone()) {
        error!("failed to send flush: {e}");
    }
}

fn send_chunk_positions(
    egress: Res<'_, EgressComm>,
    query: Query<'_, '_, (&ConnectionId, &ChunkPosition)>,
) {
    let count = query.iter().count();
    let mut stream = Vec::with_capacity(count);
    let mut positions = Vec::with_capacity(count);

    for (io, pos) in query.iter() {
        stream.push(io.inner());

        let position = hyperion_proto::ChunkPosition {
            x: pos.position.x,
            z: pos.position.y,
        };

        positions.push(position);
    }

    let packet = UpdatePlayerChunkPositions { stream, positions };

    let chunk_positions = ServerToProxyMessage::UpdatePlayerChunkPositions(packet);

    let mut v: AlignedVec = AlignedVec::new();
    // length
    v.write_u64::<byteorder::BigEndian>(0).unwrap();

    rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&chunk_positions, &mut v).unwrap();

    let len = u64::try_from(v.len() - size_of::<u64>()).unwrap();
    v[0..8].copy_from_slice(&len.to_be_bytes());

    let v = v.into_boxed_slice();
    let bytes = bytes::Bytes::from(v);

    if let Err(e) = egress.send(bytes) {
        error!("failed to send egress: {e}");
    }
}

fn broadcast_chunk_deltas(
    compose: Res<'_, Compose>,
    mut blocks: ResMut<'_, Blocks>,
    query: Query<'_, '_, &ConnectionId>,
) {
    blocks.for_each_to_update_mut(|chunk| {
        for packet in chunk.delta_drain_packets() {
            if let Err(e) = compose.broadcast(packet).send() {
                error!("failed to send chunk delta packet: {e}");
                return;
            }
        }
    });
    blocks.clear_should_update();

    for to_confirm in blocks.to_confirm.drain(..) {
        let connection_id = match query.get(to_confirm.entity) {
            Ok(connection_id) => *connection_id,
            Err(e) => {
                error!("failed to send player action response: query failed: {e}");
                continue;
            }
        };

        let pkt = PlayerActionResponseS2c {
            sequence: VarInt(to_confirm.sequence),
        };

        if let Err(e) = compose.unicast(&pkt, connection_id) {
            error!("failed to send player action response: {e}");
        }
    }
}

#[derive(Component)]
pub struct EgressPlugin;

impl Plugin for EgressPlugin {
    fn build(&self, app: &mut App) {
        let flush = {
            let flush = ServerToProxyMessage::Flush(Flush);

            let mut v: AlignedVec = AlignedVec::new();
            // length
            v.write_u64::<byteorder::BigEndian>(0).unwrap();

            rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&flush, &mut v).unwrap();

            let len = u64::try_from(v.len() - size_of::<u64>()).unwrap();
            v[0..8].copy_from_slice(&len.to_be_bytes());

            let s = Box::leak(v.into_boxed_slice());
            bytes::Bytes::from_static(s)
        };

        app.insert_resource(EncodedFlush(flush));
        app.add_systems(
            PostUpdate,
            (send_egress, send_chunk_positions, broadcast_chunk_deltas),
        );
        app.add_plugins((
            PlayerJoinPlugin,
            StatsPlugin,
            SyncChunksPlugin,
            EntityStateSyncPlugin,
        ));

        // let pipeline = world
        //     .entity()
        //     .add::<flecs::pipeline::Phase>()
        //     .depends_on::<flecs::pipeline::OnStore>();

        //         app.add_plugins((
        //             StatsPlugin,
        //             PlayerJoinPlugin,
        //             SyncChunksPlugin,
        //             EntityStateSyncPlugin,
        //         ));

        //         system!(
        //             "clear_bump",
        //             world,
        //             &mut Compose($),
        //         )
        //         .kind(pipeline)
        //         .each(move |compose| {
        //             let span = info_span!("clear_bump");
        //             let _enter = span.enter();
        //             compose.clear_bump();
        //         });
    }
}
