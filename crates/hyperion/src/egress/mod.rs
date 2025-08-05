use bevy::prelude::*;
use hyperion_proto::{ServerToProxyMessage, UpdatePlayerPositions};
use tracing::error;
use valence_protocol::{VarInt, packets::play::PlayerActionResponseS2c};

use crate::{
    Blocks,
    net::{Compose, ConnectionId},
    simulation::Position,
};
mod channel;
pub mod metadata;
pub mod player_join;
mod stats;
pub mod sync_chunks;
mod sync_entity_state;

use channel::ChannelPlugin;
use player_join::PlayerJoinPlugin;
use stats::StatsPlugin;
use sync_chunks::SyncChunksPlugin;
use sync_entity_state::EntityStateSyncPlugin;

fn send_chunk_positions(
    compose: Res<'_, Compose>,
    query: Query<'_, '_, (&ConnectionId, &Position)>,
) {
    let count = query.iter().count();
    let mut stream = Vec::with_capacity(count);
    let mut positions = Vec::with_capacity(count);

    for (io, pos) in query.iter() {
        stream.push(io.inner());
        positions.push(hyperion_proto::ChunkPosition::from(pos.to_chunk()));
    }

    let packet = UpdatePlayerPositions { stream, positions };

    let chunk_positions = ServerToProxyMessage::UpdatePlayerPositions(packet);

    compose.io_buf().add_proxy_message(&chunk_positions);
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
        app.add_systems(PostUpdate, (send_chunk_positions, broadcast_chunk_deltas));
        app.add_plugins((
            PlayerJoinPlugin,
            StatsPlugin,
            SyncChunksPlugin,
            EntityStateSyncPlugin,
            ChannelPlugin,
        ));
    }
}
