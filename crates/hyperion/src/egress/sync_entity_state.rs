use std::fmt::Debug;

use flecs_ecs::prelude::*;
use glam::{IVec3, Vec3};
use hyperion_utils::EntityExt;
use itertools::Either;
use valence_protocol::{
    ByteAngle, RawBytes, VarInt,
    packets::play::{self},
};

use crate::{
    Prev,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        Flight, MovementTracking, Owner, PendingTeleportation, Pitch, Position, Velocity, Xp, Yaw,
        animation::ActiveAnimation,
        blocks::Blocks,
        entity_kind::EntityKind,
        event::{self, HitGroundEvent},
        handlers::is_grounded,
        metadata::{MetadataChanges, get_and_clear_metadata},
    },
    spatial::get_first_collision,
    storage::Events,
};

#[derive(Component)]
pub struct EntityStateSyncModule;

fn track_previous<T: ComponentId + Copy + Debug + PartialEq>(world: &World) {
    let post_store = world
        .entity_named("post_store")
        .add::<flecs::pipeline::Phase>()
        .depends_on::<flecs::pipeline::OnStore>();

    // we include names so that if we call this multiple times, we don't get multiple observers/systems
    let component_name = std::any::type_name::<T>();

    // get the last stuff after ::
    let component_name = component_name.split("::").last().unwrap();
    let component_name = component_name.to_lowercase();

    let observer_name = format!("init_prev_{component_name}");
    let system_name = format!("track_prev_{component_name}");

    world
        .observer_named::<flecs::OnSet, &T>(&observer_name)
        .without::<(Prev, T)>() // we have not set Prev yet
        .each_entity(|entity, value| {
            entity.set_pair::<Prev, T>(*value);
        });

    world
        .system_named::<(&mut (Prev, T), &T)>(system_name.as_str())
        .multi_threaded()
        .kind_id(post_store)
        .each(|(prev, value)| {
            *prev = *value;
        });
}

impl Module for EntityStateSyncModule {
    fn module(world: &World) {
        world
            .system_named::<(
                &Compose,        // (0)
                &ConnectionId,   // (1)
                &mut (Prev, Xp), // (2)
                &mut Xp,         // (3)
            )>("entity_xp_sync")
            .term_at(0u32)
            .singleton()
            .multi_threaded()
            .kind::<flecs::pipeline::OnStore>()
            .each_iter(|table, idx, (compose, net, prev_xp, current)| {
                const {
                    assert!(size_of::<Xp>() == size_of::<u16>());
                    assert!(align_of::<Xp>() == align_of::<u16>());
                }
                let system = table.system();

                if prev_xp != current {
                    let visual = current.get_visual();

                    let packet = play::ExperienceBarUpdateS2c {
                        bar: visual.prop,
                        level: VarInt(i32::from(visual.level)),
                        total_xp: VarInt::default(),
                    };

                    let entity = table.entity(idx);
                    entity.modified::<Xp>();

                    compose.unicast(&packet, *net, system).unwrap();
                }

                *prev_xp = *current;
            });

        system!("entity_metadata_sync", world, &Compose($), &mut MetadataChanges)
            .multi_threaded()
            .kind::<flecs::pipeline::OnStore>()
            .each_iter(move |it, row, (compose, metadata_changes)| {
                let system = it.system();
                let entity = it.entity(row);
                let entity_id = VarInt(entity.minecraft_id());

                let metadata = get_and_clear_metadata(metadata_changes);

                if let Some(view) = metadata {
                    let pkt = play::EntityTrackerUpdateS2c {
                        entity_id,
                        tracked_values: RawBytes(&view),
                    };
                    if entity.has::<Position>() {
                        entity.get::<&Position>(|position| {
                            compose
                                .broadcast_local(&pkt, position.to_chunk(), system)
                                .send()
                                .unwrap();
                        });
                        return;
                    }
                    // Should never be reached but who knows
                    compose.broadcast(&pkt, system).send().unwrap();
                }
            });

        system!(
        "active_animation_sync",
        world,
        &Position,
        &Compose($),
        ?&ConnectionId,
        &mut ActiveAnimation,
        )
        .multi_threaded()
        .kind::<flecs::pipeline::OnStore>()
        .each_iter(
            move |it, row, (position, compose, connection_id, animation)| {
                let io = connection_id.copied();

                let entity = it.entity(row);
                let system = it.system();

                let entity_id = VarInt(entity.minecraft_id());

                let chunk_pos = position.to_chunk();

                for pkt in animation.packets(entity_id) {
                    compose
                        .broadcast_local(&pkt, chunk_pos, system)
                        .exclude(io)
                        .send()
                        .unwrap();
                }

                animation.clear();
            },
        );

        // What ever you do DO NOT!!! I REPEAT DO NOT SET VELOCITY ANYWHERE
        // IF YOU WANT TO APPLY VELOCITY SEND 1 VELOCITY PAKCET WHEN NEEDED LOOK in events/tag/src/module/attack.rs
        system!(
            "sync_player_entity",
            world,
            &Compose($),
            &mut Events($),
            &mut (Prev, Yaw),
            &mut (Prev, Pitch),
            &mut Position,
            &mut Velocity,
            &Yaw,
            &Pitch,
            ?&mut PendingTeleportation,
            &mut MovementTracking,
            &Flight,
        )
        .multi_threaded()
        .kind::<flecs::pipeline::PreStore>()
        .each_iter(
            |it,
             row,
             (
                compose,
                events,
                prev_yaw,
                prev_pitch,
                position,
                velocity,
                yaw,
                pitch,
                pending_teleport,
                tracking,
                flight,
            )| {
                let world = it.system().world();
                let system = it.system();
                let entity = it.entity(row);
                let entity_id = VarInt(entity.minecraft_id());

                if let Some(pending_teleport) = pending_teleport {
                    if pending_teleport.ttl == 0 {
                        entity.set::<PendingTeleportation>(PendingTeleportation::new(
                            pending_teleport.destination,
                        ));
                    }
                    pending_teleport.ttl -= 1;
                } else {
                    let chunk_pos = position.to_chunk();

                    let position_delta = **position - tracking.last_tick_position;
                    let needs_teleport = position_delta.abs().max_element() >= 8.0;
                    let changed_position = **position != tracking.last_tick_position;

                    let look_changed = (**yaw - **prev_yaw).abs() >= 0.01
                        || (**pitch - **prev_pitch).abs() >= 0.01;

                    let mut bundle = DataBundle::new(compose, system);

                    // Maximum number of movement packets allowed during 1 tick is 5
                    if tracking.received_movement_packets > 5 {
                        tracking.received_movement_packets = 1;
                    }

                    // Replace 100 by 300 if fall flying (aka elytra)
                    if f64::from(position_delta.length_squared())
                        - tracking.server_velocity.length_squared()
                        > 100f64 * f64::from(tracking.received_movement_packets)
                    {
                        entity.set(PendingTeleportation::new(tracking.last_tick_position));
                        tracking.received_movement_packets = 0;
                        return;
                    }

                    world.get::<&mut Blocks>(|blocks| {
                        let grounded = is_grounded(position, blocks);
                        tracking.was_on_ground = grounded;
                        if grounded
                            && !tracking.last_tick_flying
                            && tracking.fall_start_y - position.y > 3.
                        {
                            let event = HitGroundEvent {
                                client: *entity,
                                fall_distance: tracking.fall_start_y - position.y,
                            };
                            events.push(event, &world);
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
                    });

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

                if tracking.was_on_ground {
                    tracking.server_velocity.y = 0.;
                    #[allow(clippy::cast_possible_truncation)]
                    world.get::<&mut Blocks>(|blocks| {
                        let block_x = position.x as i32;
                        let block_y = (position.y.ceil() - 1.0) as i32; // Check the block directly below
                        let block_z = position.z as i32;

                        if let Some(state) = blocks.get_block(IVec3::new(block_x, block_y, block_z))
                        {
                            let kind = state.to_kind();
                            friction = f64::from(0.91 * kind.slipperiness() * kind.speed_factor());
                        }
                    });
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
            },
        );

        system!(
            "update_projectile_positions",
            world,
            &mut Position,
            &mut Velocity,
            &Owner,
            ?&ConnectionId
        )
        .multi_threaded()
        .kind::<flecs::pipeline::OnUpdate>()
        .with_enum_wildcard::<EntityKind>()
        .each_iter(|it, row, (position, velocity, owner, connection_id)| {
            if let Some(_connection_id) = connection_id {
                return;
            }

            let system = it.system();
            let world = system.world();
            let arrow_entity = it.entity(row);

            if velocity.0 != Vec3::ZERO {
                let center = **position;

                // getting max distance
                let distance = velocity.0.length();

                let ray = geometry::ray::Ray::new(center, velocity.0) * distance;

                let Some(collision) = get_first_collision(ray, &world, Some(owner.entity)) else {
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
                    return;
                };

                match collision {
                    Either::Left(entity) => {
                        let entity = entity.entity_view(world);
                        // send event
                        world.get::<&mut Events>(|events| {
                            events.push(
                                event::ProjectileEntityEvent {
                                    client: *entity,
                                    projectile: *arrow_entity,
                                },
                                &world,
                            );
                        });
                    }
                    Either::Right(collision) => {
                        // send event
                        world.get::<&mut Events>(|events| {
                            events.push(
                                event::ProjectileBlockEvent {
                                    collision,
                                    projectile: *arrow_entity,
                                },
                                &world,
                            );
                        });
                    }
                }
            }
        });

        track_previous::<Position>(world);
        track_previous::<Yaw>(world);
        track_previous::<Pitch>(world);
    }
}
