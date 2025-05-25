use bevy::prelude::*;
use byteorder::WriteBytesExt;
use hyperion_proto::{Flush, ServerToProxyMessage, UpdatePlayerChunkPositions};
use rkyv::util::AlignedVec;
use tracing::{error, info_span};
use valence_protocol::{VarInt, packets::play};

use crate::{net::Compose, simulation::EgressComm};
// pub mod metadata;
pub mod player_join;
// mod stats;
// pub mod sync_chunks;
// mod sync_entity_state;

use player_join::PlayerJoinPlugin;

// use stats::StatsPlugin;
// use sync_chunks::SyncChunksModule;
// use sync_entity_state::EntityStateSyncModule;
use crate::{net::ConnectionId, simulation::ChunkPosition};

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

#[derive(Component)]
pub struct EgressPlugin;

impl Plugin for EgressPlugin {
    fn build(&self, app: &mut App) {
        let world = app.world_mut();
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
        app.add_systems(PostUpdate, send_egress);
        app.add_plugins(PlayerJoinPlugin);

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

        // let player_location_query = world.new_query::<(&ConnectionId, &ChunkPosition)>();

        //         system!(
        //             "egress",
        //             world,
        //             &mut Compose($),
        //             &mut EgressComm($),
        //         )
        //         .kind(pipeline)
        //         .each(move |(compose, egress)| {
        //             let span = info_span!("egress");
        //             let _enter = span.enter();
        //
        //             {
        //                 let span = info_span!("chunk_positions");
        //                 let _enter = span.enter();
        //
        //                 let mut stream = Vec::new();
        //                 let mut positions = Vec::new();
        //
        //                 player_location_query.each(|(io, pos)| {
        //                     stream.push(io.inner());
        //
        //                     let position = hyperion_proto::ChunkPosition {
        //                         x: pos.position.x,
        //                         z: pos.position.y,
        //                     };
        //
        //                     positions.push(position);
        //                 });
        //
        //                 let packet = UpdatePlayerChunkPositions { stream, positions };
        //
        //                 let chunk_positions = ServerToProxyMessage::UpdatePlayerChunkPositions(packet);
        //
        //                 let mut v: AlignedVec = AlignedVec::new();
        //                 // length
        //                 v.write_u64::<byteorder::BigEndian>(0).unwrap();
        //
        //                 rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&chunk_positions, &mut v)
        //                     .unwrap();
        //
        //                 let len = u64::try_from(v.len() - size_of::<u64>()).unwrap();
        //                 v[0..8].copy_from_slice(&len.to_be_bytes());
        //
        //                 let v = v.into_boxed_slice();
        //                 let bytes = bytes::Bytes::from(v);
        //
        //                 if let Err(e) = egress.send(bytes) {
        //                     error!("failed to send egress: {e}");
        //                 }
        //             }
        //        });

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

// chunk deltas
// fn x(compose: Res<'_, Compose>, mut mc: ResMut<'_, Blocks>) {
//     mc.for_each_to_update_mut(|chunk| {
//         for packet in chunk.delta_drain_packets() {
//             if let Err(e) = compose.broadcast(packet).send() {
//                 error!("failed to send chunk delta packet: {e}");
//                 return;
//             }
//         }
//     });
//     mc.clear_should_update();
//
//     for to_confirm in mc.to_confirm.drain(..) {
//         let entity = world.entity_from_id(to_confirm.entity);
//
//         let pkt = play::PlayerActionResponseS2c {
//             sequence: VarInt(to_confirm.sequence),
//         };
//
//         entity.get::<&ConnectionId>(|stream| {
//             if let Err(e) = compose.unicast(&pkt, *stream) {
//                 error!("failed to send player action response: {e}");
//             }
//         });
//     }
// }
