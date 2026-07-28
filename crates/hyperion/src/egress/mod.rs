use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::clientbound::BlockChangedAck,
};
use tracing::{error, info_span};

use crate::{
    net::{
        Compose, ConnectionId,
        intermediate::{
            IntermediateServerToProxyMessage, UpdatePlayerPosition, UpdatePlayerPositions,
        },
        protocol::send,
    },
    simulation::{Position, blocks::Blocks},
};

pub mod boss_bar;
mod channel;
pub mod metadata;
pub mod player_join;
pub mod server_load;
mod stats;
pub mod sync_chunks;
mod sync_entity_state;

use boss_bar::BossBarModule;
use channel::ChannelModule;
use player_join::PlayerJoinModule;
use server_load::ServerLoadModule;
use stats::StatsModule;
use sync_chunks::SyncChunksModule;
use sync_entity_state::EntityStateSyncModule;

#[derive(Component)]
pub struct EgressModule;

impl Module for EgressModule {
    fn module(world: &World) {
        let pipeline = world
            .entity()
            .add(id::<flecs::pipeline::Phase>())
            .depends_on(id::<flecs::pipeline::OnStore>());

        world.import::<StatsModule>();
        world.import::<PlayerJoinModule>();
        world.import::<SyncChunksModule>();
        world.import::<EntityStateSyncModule>();
        world.import::<ChannelModule>();
        world.import::<BossBarModule>();
        world.import::<ServerLoadModule>();

        system!("broadcast_chunk_deltas", world, &Compose, &mut Blocks,)
            .kind(id::<flecs::pipeline::OnUpdate>())
            .each_iter(move |it: TableIter<'_, false>, _, (compose, mc)| {
                let span = info_span!("broadcast_chunk_deltas");
                let _enter = span.enter();

                let world = it.world();

                mc.for_each_to_update_mut(|chunk| {
                    for packet in chunk.delta_drain_packets() {
                        if let Err(e) = compose.broadcast(packet).send() {
                            error!("failed to send chunk delta packet: {e}");
                            return;
                        }
                    }
                });
                mc.clear_should_update();

                for to_confirm in mc.to_confirm.drain(..) {
                    let entity = world.entity_from_id(to_confirm.entity);

                    let pkt = BlockChangedAck(to_confirm.sequence);

                    entity.get::<&ConnectionId>(|stream| {
                        if let Err(e) =
                            send(compose, *stream, PacketId::BlockChangedAck.to_raw(), &pkt)
                        {
                            error!("failed to send player action response: {e}");
                        }
                    });
                }
            });

        let player_location_query = world.new_query::<(&ConnectionId, &Position)>();

        system!("send_chunk_positions", world, &Compose,)
            .kind(pipeline)
            .each(move |compose| {
                let span = info_span!("send_chunk_positions");
                let _enter = span.enter();

                let count = player_location_query.count();
                let count = usize::try_from(count).unwrap_or_default();

                let mut players = Vec::with_capacity(count);

                player_location_query.each(|(&io, pos)| {
                    players.push(UpdatePlayerPosition {
                        stream: io,
                        position: hyperion_proxy_proto::ChunkPosition::from(pos.to_chunk()),
                    });
                });

                let packet = UpdatePlayerPositions { players };

                compose.io_buf().add_proxy_message(
                    &IntermediateServerToProxyMessage::UpdatePlayerPositions(packet),
                );
            });
    }
}
