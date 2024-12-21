use std::{borrow::Cow, fmt::Debug};

use anyhow::Context;
use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion_inventory::PlayerInventory;
use hyperion_utils::EntityExt;
use itertools::Either;
use tracing::{debug, error};
use valence_protocol::{
    ByteAngle, RawBytes, VarInt,
    packets::play::{self, entity_equipment_update_s2c::EquipmentEntry},
};
use valence_server::BlockState;

use crate::{
    Prev,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        Pitch, Position, Velocity, Xp, Yaw,
        animation::ActiveAnimation,
        blocks::Blocks,
        entity_kind::EntityKind,
        event,
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
            .run(|mut table| {
                let system = table.system();
                while table.next() {
                    let count = table.count();

                    unsafe {
                        const _: () = assert!(size_of::<Xp>() == size_of::<u16>());
                        const _: () = assert!(align_of::<Xp>() == align_of::<u16>());

                        /// Number of lanes in the SIMD vector
                        const LANES: usize = 32; // up to AVX512

                        let compose = table.field_unchecked::<Compose>(0);
                        let compose = compose.first().unwrap();

                        let net = table.field_unchecked::<ConnectionId>(1);
                        let net = net.get(..).unwrap();

                        let mut prev_xp = table.field_unchecked::<Xp>(2);
                        let prev_xp = prev_xp.get_mut(..).unwrap();
                        let prev_xp: &mut [u16] =
                            core::slice::from_raw_parts_mut(prev_xp.as_mut_ptr().cast(), count);

                        let mut xp = table.field_unchecked::<Xp>(3);
                        let xp = xp.get_mut(..).unwrap();
                        let xp: &mut [u16] =
                            core::slice::from_raw_parts_mut(xp.as_mut_ptr().cast(), count);

                        simd_utils::copy_and_get_diff::<_, LANES>(
                            prev_xp,
                            xp,
                            |idx, prev, current| {
                                debug_assert!(prev != current);

                                let net = net.get(idx).unwrap();

                                let current = Xp::from(*current);
                                let visual = current.get_visual();

                                let packet = play::ExperienceBarUpdateS2c {
                                    bar: visual.prop,
                                    level: VarInt(i32::from(visual.level)),
                                    total_xp: VarInt::default(),
                                };

                                let entity = table.entity(idx);
                                entity.modified::<Xp>();

                                compose.unicast(&packet, *net, system).unwrap();
                            },
                        );
                    }
                }
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

                    // todo(perf): do so locally
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

        system!(
            "player_inventory_sync",
            world,
            &Compose($),
            &mut PlayerInventory,
            &ConnectionId,
        )
        .multi_threaded()
        .kind::<flecs::pipeline::OnStore>()
        .each_iter(move |it, _, (compose, inventory, io)| {
            let mut run = || {
                let io = *io;
                let system = it.system();

                for slot in &inventory.updated_since_last_tick {
                    let Ok(slot) = u16::try_from(slot) else {
                        error!("failed to convert slot to u16 {slot}");
                        continue;
                    };
                    let item = inventory
                        .get(slot)
                        .with_context(|| format!("failed to get item for slot {slot}"))?;
                    let Ok(slot) = i16::try_from(slot) else {
                        error!("failed to convert slot to i16 {slot}");
                        continue;
                    };
                    let pkt = play::ScreenHandlerSlotUpdateS2c {
                        window_id: 0,
                        state_id: VarInt::default(),
                        slot_idx: slot,
                        slot_data: Cow::Borrowed(item),
                    };
                    compose
                        .unicast(&pkt, io, system)
                        .context("failed to send inventory update")?;
                }

                inventory.updated_since_last_tick.clear();
                inventory.hand_slot_updated_since_last_tick = false;

                anyhow::Ok(())
            };

            if let Err(e) = run() {
                error!("Failed to sync player inventory: {}", e);
            }
        });

        system!(
            "sync_equipped_items",
            world,
            &Compose($),
            &Position,
            &PlayerInventory,
        )
        .multi_threaded()
        .kind::<flecs::pipeline::OnStore>()
        .each_iter(move |it, row, (compose, position, inventory)| {
            // let entity = it.entity(row);
            let system = it.system();
            // get armor and hand
            let hand = EquipmentEntry {
                slot: 0,
                item: inventory.get_cursor().clone(),
            };
            let helmet = EquipmentEntry {
                slot: 5,
                item: inventory.get_helmet().clone(),
            };
            let chestplate = EquipmentEntry {
                slot: 4,
                item: inventory.get_chestplate().clone(),
            };
            let leggings = EquipmentEntry {
                slot: 3,
                item: inventory.get_leggings().clone(),
            };
            let boots = EquipmentEntry {
                slot: 2,
                item: inventory.get_boots().clone(),
            };
            let off_hand = EquipmentEntry {
                slot: 1,
                item: inventory.get_offhand().clone(),
            };

            let packet = play::EntityEquipmentUpdateS2c {
                entity_id: VarInt(it.entity(row).minecraft_id()),
                equipment: vec![hand, helmet, chestplate, leggings, boots, off_hand],
            };

            compose
                .broadcast_local(&packet, position.to_chunk(), system)
                .send()
                .unwrap();
        });

        // What ever you do DO NOT!!! I REPEAT DO NOT SET VELOCITY ANYWHERE
        // IF YOU WANT TO APPLY VELOCITY SEND 1 VELOCITY PAKCET WHEN NEEDED LOOK in events/tag/src/module/attack.rs
        system!(
            "sync_player_entity",
            world,
            &Compose($),
            &mut (Prev, Position),
            &mut (Prev, Yaw),
            &mut (Prev, Pitch),
            &mut Position,
            &mut Velocity,
            &Yaw,
            &Pitch,
        )
        .multi_threaded()
        .kind::<flecs::pipeline::PreStore>()
        .each_iter(
            |it,
             row,
             (
                compose,
                prev_position,
                prev_yaw,
                prev_pitch,
                position,
                velocity,
                yaw,
                pitch,
            )| {
                // if io.is_none() {
                // return;
                // }

                let world = it.system().world();
                let system = it.system();
                let entity = it.entity(row);
                let entity_id = VarInt(entity.minecraft_id());

                let chunk_pos = position.to_chunk();

                let position_delta = **position - **prev_position;
                let needs_teleport = position_delta.abs().max_element() >= 8.0;
                let changed_position = **position != **prev_position;

                let look_changed = (**yaw - **prev_yaw).abs() >= 0.01 || (**pitch - **prev_pitch).abs() >= 0.01;

                let mut bundle = DataBundle::new(compose, system);

                world.get::<&mut Blocks>(|blocks| {
                    let grounded = is_grounded(position, blocks);

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
                }

                bundle.broadcast_local(chunk_pos).unwrap();
            },
        );

        system!(
            "update_projectile_positions",
            world,
            &mut Position,
            &mut Velocity,
            ?&ConnectionId
        )
        .multi_threaded()
        .kind::<flecs::pipeline::OnUpdate>()
        .with_enum_wildcard::<EntityKind>()
        .each_iter(|it, row, (position, velocity, connection_id)| {
            if let Some(_connection_id) = connection_id {
                return;
            }

            let system = it.system();
            let world = system.world();
            let _entity = it.entity(row);

            if velocity.0 != Vec3::ZERO {
                // let (new_yaw, new_pitch) = get_rotation_from_velocity(velocity.0);
                let center = **position;

                // getting max distance
                let distance = velocity.0.length();

                let ray = geometry::ray::Ray::new(center, velocity.0) * distance;

                let Some(collision) = get_first_collision(ray, &world) else {
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
                        world.get::<&mut Events>(|events| events.push(
                            event::ProjectileEntityEvent {
                                client: *entity,
                                projectile: *_entity,
                            },
                            &world
                        ));
                    }
                    Either::Right(collision) => {
                        // send event
                        world.get::<&mut Events>(|events| events.push(
                            event::ProjectileBlockEvent {
                                collision: collision,
                                projectile: *_entity,
                            },
                            &world
                        ));
                    }
                }

                /* debug!("collision = {collision:?}");

                velocity.0 = Vec3::ZERO; */

                /* // Set arrow position to the collision location
                **position = collision.normal;

                blocks
                    .set_block(collision.location, BlockState::DIRT)
                    .unwrap(); */
            }
        });

        track_previous::<Position>(world);
        track_previous::<Yaw>(world);
        track_previous::<Pitch>(world);
    }
}
