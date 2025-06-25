use bevy::prelude::*;
use glam::{IVec3, Vec3};
use hyperion_utils::{EntityExt, Prev, track_prev};
use itertools::Either;
use tracing::error;
use valence_bytes::CowBytes;
use valence_protocol::{
    ByteAngle, RawBytes, VarInt,
    packets::play::{self},
};

use crate::{
    Blocks,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        EntitySize, Flight, MovementTracking, Owner, PendingTeleportation, Pitch, Position,
        Velocity, Xp, Yaw,
        animation::ActiveAnimation,
        event,
        event::HitGroundEvent,
        handlers::is_grounded,
        metadata::{MetadataChanges, get_and_clear_metadata},
    },
    spatial::{SpatialIndex, get_first_collision},
};

pub struct EntityStateSyncPlugin;

fn entity_xp_sync(
    compose: Res<'_, Compose>,
    query: Query<'_, '_, (&ConnectionId, &Prev<Xp>, &Xp)>,
) {
    for (&connection_id, prev_xp, current) in query.iter() {
        if **prev_xp != *current {
            let visual = current.get_visual();

            let packet = play::ExperienceBarUpdateS2c {
                bar: visual.prop,
                level: VarInt(i32::from(visual.level)),
                total_xp: VarInt::default(),
            };

            compose.unicast(&packet, connection_id).unwrap();
        }
    }
}

fn entity_metadata_sync(
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (Entity, &Position, &mut MetadataChanges)>,
) {
    for (entity_id, position, mut metadata_changes) in &mut query {
        let metadata = get_and_clear_metadata(&mut metadata_changes);

        if let Some(view) = metadata {
            let pkt = play::EntityTrackerUpdateS2c {
                entity_id: VarInt(entity_id.minecraft_id()),
                tracked_values: RawBytes(CowBytes::Borrowed(&view)),
            };
            compose
                .broadcast_local(&pkt, position.to_chunk())
                .send()
                .unwrap();
        }
    }
}

fn active_animation_sync(
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (Entity, &Position, &ConnectionId, &mut ActiveAnimation)>,
) {
    for (entity, position, &connection_id, mut animation) in &mut query {
        let entity_id = VarInt(entity.minecraft_id());

        let chunk_pos = position.to_chunk();

        for pkt in animation.packets(entity_id) {
            compose
                .broadcast_local(&pkt, chunk_pos)
                .exclude(Some(connection_id))
                .send()
                .unwrap();
        }

        animation.clear();
    }
}

/// What ever you do DO NOT!!! I REPEAT DO NOT SET VELOCITY ANYWHERE
/// IF YOU WANT TO APPLY VELOCITY SEND 1 VELOCITY PAKCET WHEN NEEDED LOOK in events/tag/src/module/attack.rs
fn sync_player_entity(
    compose: Res<'_, Compose>,
    blocks: Res<'_, Blocks>,
    mut query: Query<
        '_,
        '_,
        (
            Entity,
            &Prev<Yaw>,
            &Prev<Pitch>,
            &Position,
            &mut Velocity,
            &Yaw,
            &Pitch,
            Option<&mut PendingTeleportation>,
            &mut MovementTracking,
            &Flight,
        ),
    >,
    mut event_writer: EventWriter<'_, HitGroundEvent>,
    mut commands: Commands<'_, '_>,
) {
    for (
        entity,
        prev_yaw,
        prev_pitch,
        position,
        mut velocity,
        yaw,
        pitch,
        pending_teleport,
        mut tracking,
        flight,
    ) in &mut query
    {
        let entity_id = VarInt(entity.minecraft_id());

        if let Some(mut pending_teleport) = pending_teleport {
            if pending_teleport.ttl == 0 {
                // This needs to trigger OnInsert, so pending_teleport cannot be modified directly
                commands
                    .entity(entity)
                    .insert(PendingTeleportation::new(pending_teleport.destination));
            } else {
                pending_teleport.ttl -= 1;
            }
        } else {
            let chunk_pos = position.to_chunk();

            let position_delta = **position - tracking.last_tick_position;
            let needs_teleport = position_delta.abs().max_element() >= 8.0;
            let changed_position = **position != tracking.last_tick_position;

            let look_changed =
                (**yaw - ***prev_yaw).abs() >= 0.01 || (**pitch - ***prev_pitch).abs() >= 0.01;

            let mut bundle = DataBundle::new(&compose);

            // Maximum number of movement packets allowed during 1 tick is 5
            if tracking.received_movement_packets > 5 {
                tracking.received_movement_packets = 1;
            }

            // Replace 100 by 300 if fall flying (aka elytra)
            if f64::from(position_delta.length_squared())
                - tracking.server_velocity.length_squared()
                > 100f64 * f64::from(tracking.received_movement_packets)
            {
                commands
                    .entity(entity)
                    .insert(PendingTeleportation::new(tracking.last_tick_position));
                tracking.received_movement_packets = 0;
                return;
            }

            let grounded = is_grounded(position, &blocks);
            tracking.was_on_ground = grounded;
            if grounded && !tracking.last_tick_flying && tracking.fall_start_y - position.y > 3. {
                let event = HitGroundEvent {
                    client: entity,
                    fall_distance: tracking.fall_start_y - position.y,
                };
                event_writer.write(event);
                tracking.fall_start_y = position.y;
            }

            if (tracking.last_tick_flying && flight.allow) || position_delta.y >= 0. {
                tracking.fall_start_y = position.y;
            }

            if changed_position && !needs_teleport && look_changed {
                let packet = play::RotateAndMoveRelativeS2c {
                    entity_id,
                    #[allow(clippy::cast_possible_truncation)]
                    delta: (position_delta * 4096.0).to_array().map(|x| x as i16),
                    yaw: ByteAngle::from_degrees(**yaw),
                    pitch: ByteAngle::from_degrees(**pitch),
                    on_ground: grounded,
                };

                bundle.add_packet(&packet).unwrap();
            } else {
                if changed_position && !needs_teleport {
                    let packet = play::MoveRelativeS2c {
                        entity_id,
                        #[allow(clippy::cast_possible_truncation)]
                        delta: (position_delta * 4096.0).to_array().map(|x| x as i16),
                        on_ground: grounded,
                    };

                    bundle.add_packet(&packet).unwrap();
                }

                if look_changed {
                    let packet = play::RotateS2c {
                        entity_id,
                        yaw: ByteAngle::from_degrees(**yaw),
                        pitch: ByteAngle::from_degrees(**pitch),
                        on_ground: grounded,
                    };

                    bundle.add_packet(&packet).unwrap();
                }
                let packet = play::EntitySetHeadYawS2c {
                    entity_id,
                    head_yaw: ByteAngle::from_degrees(**yaw),
                };

                bundle.add_packet(&packet).unwrap();
            }

            if needs_teleport {
                let packet = play::EntityPositionS2c {
                    entity_id,
                    position: position.as_dvec3(),
                    yaw: ByteAngle::from_degrees(**yaw),
                    pitch: ByteAngle::from_degrees(**pitch),
                    on_ground: grounded,
                };

                bundle.add_packet(&packet).unwrap();
            }

            if velocity.0 != Vec3::ZERO {
                let packet = play::EntityVelocityUpdateS2c {
                    entity_id,
                    velocity: velocity.to_packet_units(),
                };

                bundle.add_packet(&packet).unwrap();
                velocity.0 = Vec3::ZERO;
            }

            bundle.broadcast_local(chunk_pos).unwrap();
        }

        tracking.received_movement_packets = 0;
        tracking.last_tick_position = **position;
        tracking.last_tick_flying = flight.is_flying;

        let mut friction = 0.91;

        #[allow(clippy::cast_possible_truncation)]
        if tracking.was_on_ground {
            tracking.server_velocity.y = 0.;
            let block_x = position.x as i32;
            let block_y = (position.y.ceil() - 1.0) as i32; // Check the block directly below
            let block_z = position.z as i32;

            if let Some(state) = blocks.get_block(IVec3::new(block_x, block_y, block_z)) {
                let kind = state.to_kind();
                friction = f64::from(0.91 * kind.slipperiness() * kind.speed_factor());
            }
        }

        tracking.server_velocity.x *= friction * 0.98;
        tracking.server_velocity.y -= 0.08 * 0.980_000_019_073_486_3;
        tracking.server_velocity.z *= friction * 0.98;

        if tracking.server_velocity.x.abs() < 0.003 {
            tracking.server_velocity.x = 0.;
        }

        if tracking.server_velocity.y.abs() < 0.003 {
            tracking.server_velocity.y = 0.;
        }

        if tracking.server_velocity.z.abs() < 0.003 {
            tracking.server_velocity.z = 0.;
        }
    }
}

fn update_projectile_positions(
    arrow_query: Query<'_, '_, (Entity, &Owner)>,
    mut query_set: ParamSet<
        '_,
        '_,
        (
            Query<'_, '_, (&mut Position, &mut Velocity)>,
            Query<'_, '_, (&Position, &EntitySize)>,
        ),
    >,
    mut projectile_block_writer: EventWriter<'_, event::ProjectileBlockEvent>,
    mut projectile_entity_writer: EventWriter<'_, event::ProjectileEntityEvent>,
    index: Res<'_, SpatialIndex>,
    blocks: Res<'_, Blocks>,
) {
    for (arrow_entity, owner) in arrow_query.iter() {
        let pv_query = query_set.p0();
        let (position, velocity) = match pv_query.get(arrow_entity) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to update projectile positions: query failed: {e}");
                continue;
            }
        };

        if velocity.0 == Vec3::ZERO {
            continue;
        }

        let center = **position;

        // getting max distance
        let distance = velocity.0.length();

        let ray = geometry::ray::Ray::new(center, velocity.0) * distance;

        match get_first_collision(ray, &index, &blocks, query_set.p1(), Some(owner.entity)) {
            Some(Either::Left(entity)) => {
                // send event
                projectile_entity_writer.write(event::ProjectileEntityEvent {
                    client: entity,
                    projectile: arrow_entity,
                });
            }
            Some(Either::Right(collision)) => {
                // send event
                projectile_block_writer.write(event::ProjectileBlockEvent {
                    collision,
                    projectile: arrow_entity,
                });
            }
            None => {
                let mut pv_query = query_set.p0();
                let (mut position, mut velocity) = match pv_query.get_mut(arrow_entity) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("failed to update projectile positions: query failed: {e}");
                        continue;
                    }
                };

                // Drag (0.99 / 20.0)
                // 1.0 - (0.99 / 20.0) * 0.05
                velocity.0 *= 0.997_525;

                // Gravity (20 MPSS)
                velocity.0.y -= 0.05;

                // Terminal Velocity max (100.0)
                velocity.0 = velocity.0.clamp_length_max(100.0);

                position.x += velocity.0.x;
                position.y += velocity.0.y;
                position.z += velocity.0.z;
            }
        }
    }
}

impl Plugin for EntityStateSyncPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedPostUpdate,
            (
                entity_xp_sync,
                entity_metadata_sync,
                active_animation_sync,
                sync_player_entity,
                update_projectile_positions,
            ),
        );

        track_prev::<Xp>(app);
        track_prev::<Position>(app);
        track_prev::<Yaw>(app);
        track_prev::<Pitch>(app);
    }
}
