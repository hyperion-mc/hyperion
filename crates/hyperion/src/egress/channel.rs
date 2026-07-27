use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::entity::SetEntityData,
};
use hyperion_proto::UpdateChannelPosition;
use hyperion_utils::EntityExt;
use tracing::error;
use valence_protocol::{VarInt, packets::play};

use crate::{
    egress::metadata::show_all,
    net::{
        Channel, ChannelId, Compose, ConnectionId,
        intermediate::{IntermediateServerToProxyMessage, UpdateChannelPositions},
        protocol::Clientbound,
    },
    simulation::{
        Pitch, Position, Uuid, Velocity, Yaw, add_entity,
        entity_kind::EntityKind,
        event::RequestSubscribeChannelPackets,
        metadata::{MetadataChanges, get_and_clear_metadata},
    },
    storage::EventQueue,
};

#[derive(Component)]
pub struct ChannelModule;

impl Module for ChannelModule {
    fn module(world: &World) {
        world
            .observer::<flecs::OnAdd, ()>()
            .with(id::<Channel>())
            .each_entity(|entity, ()| {
                let packet = play::EntitiesDestroyS2c {
                    entity_ids: vec![VarInt(entity.minecraft_id())].into(),
                };

                entity.world().get::<&Compose>(|compose| {
                    let packet_buf = compose.io_buf().encode_packet(&packet, compose).unwrap();

                    compose
                        .io_buf()
                        .add_channel(ChannelId::from(entity), &packet_buf);
                });
            });

        world
            .observer::<flecs::OnRemove, ()>()
            .with(id::<Channel>())
            .each_entity(|entity, ()| {
                entity.world().get::<&Compose>(|compose| {
                    compose.io_buf().remove_channel(ChannelId::from(entity));
                });
            });

        let channels = world.query::<&Position>().with(id::<Channel>()).build();

        system!("update_channel_positions", world, &Compose)
            .kind(id::<flecs::pipeline::OnUpdate>())
            .each(move |compose| {
                let mut updates = Vec::new();

                channels.each_entity(|entity, position| {
                    updates.push(UpdateChannelPosition {
                        channel_id: ChannelId::from(entity).inner(),
                        position: position.to_chunk().into(),
                    });
                });

                compose.io_buf().add_proxy_message(
                    &IntermediateServerToProxyMessage::UpdateChannelPositions(
                        UpdateChannelPositions { updates: &updates },
                    ),
                );
            });

        system!(
            "send_subscribe_channel_packets",
            world,
            &Compose,
            &mut EventQueue<RequestSubscribeChannelPackets>,
        )
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(|it: TableIter<'_, false>, _, (compose, event_queue)| {
            let world = it.world();

            for RequestSubscribeChannelPackets(channel) in event_queue.drain() {
                let Some(entity) = world.try_get_alive(channel) else {
                    error!("failed to send subscribe channel packets: entity is not alive");
                    continue;
                };

                let Some(entity_kind) = entity
                    .target(id::<EntityKind>(), 0)
                    .map(|constant| constant.to_constant::<EntityKind>())
                else {
                    error!("failed to send subscribe channel packets: entity has no entity kind");
                    continue;
                };

                let minecraft_id = entity.minecraft_id();

                let found = entity.try_get::<(
                    &Uuid,
                    &Position,
                    &Pitch,
                    &Yaw,
                    &Velocity,
                    Option<&ConnectionId>,
                )>(
                    |(uuid, position, pitch, yaw, velocity, connection_id)| {
                        // A player is spawned with `add_entity` like anything
                        // else since 1.20.2; `player_spawn` no longer exists.
                        let spawn_packet = add_entity(
                            minecraft_id,
                            entity_kind,
                            uuid,
                            position,
                            *pitch,
                            *yaw,
                            velocity,
                        );
                        let mut packet_buf = compose
                            .io_buf()
                            .encode_packet(
                                Clientbound::new(PacketId::AddEntity.to_raw(), &spawn_packet),
                                compose,
                            )
                            .unwrap();

                        if entity_kind == EntityKind::Player {
                            let show_all = show_all(minecraft_id);
                            packet_buf.extend_from_slice(
                                &compose
                                    .io_buf()
                                    .encode_packet(
                                        Clientbound::new(
                                            PacketId::SetEntityData.to_raw(),
                                            &show_all,
                                        ),
                                        compose,
                                    )
                                    .unwrap(),
                            );
                        }

                        let mut metadata = MetadataChanges::default();
                        metadata.encode_non_default_components(entity);

                        if let Some(view) = get_and_clear_metadata(&mut metadata) {
                            let pkt = SetEntityData {
                                id: minecraft_id,
                                packed_items: &view,
                            };
                            packet_buf.extend_from_slice(
                                &compose
                                    .io_buf()
                                    .encode_packet(
                                        Clientbound::new(PacketId::SetEntityData.to_raw(), &pkt),
                                        compose,
                                    )
                                    .unwrap(),
                            );
                        }

                        compose.io_buf().send_subscribe_channel_packets(
                            ChannelId::from(entity),
                            &packet_buf,
                            connection_id.copied(),
                        );
                    },
                );

                if found.is_none() {
                    error!(
                        "failed to send subscribe channel packets: entity is missing required \
                         components"
                    );
                }
            }
        });
    }
}
