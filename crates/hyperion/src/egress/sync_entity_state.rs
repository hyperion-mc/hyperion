//! Keeping every observer's view of an entity up to date.
//!
//! Movement is sent as a delta where it fits and as a position sync where it
//! does not, the same choice `net.minecraft.server.level.ServerEntity` makes:
//! the delta is `round(x * 4096)` differenced against the last value sent, so
//! it saturates a `short` at eight blocks and the entity has to be resynced
//! instead.

use std::fmt::Debug;

use flecs_ecs::prelude::*;
use glam::{IVec3, Vec3};
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId,
    packets::play::{
        clientbound::SetExperience,
        entity::{
            EntityPositionSync, MoveEntityPos, MoveEntityPosRot, MoveEntityRot, RotateHead,
            SetEntityData, SetEntityMotion, pack_degrees,
        },
    },
    types::{PositionMoveRotation, Vec3 as ProtoVec3},
};
use hyperion_utils::EntityExt;
use itertools::Either;

use crate::{
    Prev,
    net::{Channel, ChannelId, Compose, ConnectionId, DataBundle, protocol::Clientbound},
    simulation::{
        BroadcastProjectile, Flight, MovementTracking, Owner, PendingTeleportation, Pitch,
        Position, Velocity, Xp, Yaw,
        animation::ActiveAnimation,
        blocks::Blocks,
        entity_kind::EntityKind,
        event::{self, HitGroundEvent},
        handlers::is_grounded,
        metadata::{MetadataChanges, get_and_clear_metadata},
        projectile_motion::{MotionOrder, ProjectileMotion, lerp_rotation, look_angles},
    },
    spatial::get_first_collision,
    storage::Events,
};

/// Broadcasts a projectile's current velocity to every client in its channel,
/// as one `SetEntityMotion`, every tick.
///
/// Not an absolute `EntityPositionSync`: an arrow has no client-side
/// interpolation handler (`AbstractArrow` does not override `getInterpolation`),
/// so `handleEntityPositionSync` -> `setPos` HARD-TELEPORTS it -- one absolute
/// sync per tick snaps the arrow ~3 blocks every 50ms, which is the jagged
/// motion. The client already dead-reckons the arrow's own physics each tick
/// (`AbstractArrow.tick`: `pos += vel; vel *= 0.99; vel.y -= 0.05`), seeded from
/// the `add_entity` velocity, and it applies `SetEntityMotion` through
/// `lerpMotion` -- so re-sending the server's velocity each tick keeps the
/// client's prediction exact and it renders smoothly, the way vanilla does
/// (velocity on change plus client prediction, never a per-tick teleport). The
/// heading is re-derived client-side from the velocity, so it is not sent.
fn broadcast_projectile_velocity(compose: &Compose, channel: ChannelId, id: i32, velocity: Vec3) {
    let packet = SetEntityMotion {
        id,
        movement: ProtoVec3 {
            x: f64::from(velocity.x),
            y: f64::from(velocity.y),
            z: f64::from(velocity.z),
        },
    };
    if let Err(error) = compose
        .broadcast_channel(
            Clientbound::new(PacketId::SetEntityMotion.to_raw(), &packet),
            channel,
        )
        .send()
    {
        tracing::error!("failed to broadcast arrow velocity: {error}");
    }
}

/// Eases a projectile's stored facing toward the heading of `velocity`, in
/// vanilla's projectile convention (see [`look_angles`]), and returns the yaw
/// and pitch it settled on so the caller can put them on the wire. `None` for
/// an entity carrying neither a yaw nor a pitch, which is anything the physics
/// module does not aim.
fn aim_along(
    yaw: Option<&mut Yaw>,
    pitch: Option<&mut Pitch>,
    velocity: Vec3,
) -> Option<(f32, f32)> {
    let (yaw, pitch) = (yaw?, pitch?);
    let (target_yaw, target_pitch) = look_angles(velocity);
    **yaw = lerp_rotation(**yaw, target_yaw);
    **pitch = lerp_rotation(**pitch, target_pitch);
    Some((**yaw, **pitch))
}

#[derive(Component)]
pub struct EntityStateSyncModule;

/// The movement delta `ClientboundMoveEntityPacket` carries.
///
/// `VecDeltaCodec` rounds each coordinate to 1/4096 of a block *before*
/// subtracting, rather than rounding the difference, so a stationary entity
/// whose position is not exactly representable does not drift by one step per
/// tick. Saturating rather than wrapping is this server's own choice: the
/// caller only sends a delta it has already checked fits, and wrapping a
/// coordinate that slipped through would teleport the entity to the far side
/// of the range instead of to its edge.
fn encode_delta(from: Vec3, to: Vec3) -> [i16; 3] {
    let step = |old: f32, new: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a coordinate is inside the world limit, so 4096 times it is far inside i64"
        )]
        let quantise = |value: f32| (f64::from(value) * 4096.0).round() as i64;
        i16::try_from(quantise(new) - quantise(old)).unwrap_or(i16::MAX)
    };
    [step(from.x, to.x), step(from.y, to.y), step(from.z, to.z)]
}

fn track_previous<T: ComponentId + Copy + Debug + PartialEq>(world: &World) {
    let post_store = world
        .entity_named("post_store")
        .add(id::<flecs::pipeline::Phase>())
        .depends_on(id::<flecs::pipeline::OnStore>());

    // we include names so that if we call this multiple times, we don't get multiple observers/systems
    let component_name = std::any::type_name::<T>();

    // get the last stuff after ::
    let component_name = component_name.split("::").last().unwrap();
    let component_name = component_name.to_lowercase();

    let observer_name = format!("init_prev_{component_name}");
    let system_name = format!("track_prev_{component_name}");

    // flecs 0.2.2 cannot infer the pair kind for a generic `(Id<Prev>, Id<T>)`, so build the pair
    // id by hand
    let prev_pair = Entity(ecs_pair(
        *world.component_id::<Prev>(),
        *world.component_id::<T>(),
    ));

    world
        .observer_named::<flecs::OnSet, &T>(&observer_name)
        .without(prev_pair) // we have not set Prev yet
        .each_entity(|entity, value| {
            entity.set_pair::<Prev, T>(*value);
        });

    world
        .system_named::<(&mut (Prev, T), &T)>(system_name.as_str())
        .kind(post_store)
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
            .kind(id::<flecs::pipeline::OnStore>())
            .each_iter(|table, idx, (compose, net, prev_xp, current)| {
                const {
                    assert!(size_of::<Xp>() == size_of::<u16>());
                    assert!(align_of::<Xp>() == align_of::<u16>());
                }
                if prev_xp != current {
                    let visual = current.get_visual();

                    let packet = SetExperience {
                        experience_progress: visual.prop,
                        experience_level: i32::from(visual.level),
                        // The client only renders the level and the bar, so
                        // the running total it would use for a death drop is
                        // left at zero rather than tracked.
                        total_experience: 0,
                    };

                    let entity = table.entity(idx);
                    entity.modified(id::<Xp>());

                    compose
                        .unicast(
                            Clientbound::new(PacketId::SetExperience.to_raw(), &packet),
                            *net,
                        )
                        .unwrap();
                }

                *prev_xp = *current;
            });

        system!(
            "entity_metadata_sync",
            world,
            &Compose,
            &mut MetadataChanges
        )
        .kind(id::<flecs::pipeline::OnStore>())
        .each_iter(move |it, row, (compose, metadata_changes)| {
            let entity = it.entity(row);
            let entity_id = entity.minecraft_id();

            let metadata = get_and_clear_metadata(metadata_changes);

            if let Some(view) = metadata {
                let pkt = SetEntityData {
                    id: entity_id,
                    packed_items: &view,
                };
                compose
                    .broadcast_channel(
                        Clientbound::new(PacketId::SetEntityData.to_raw(), &pkt),
                        entity.into(),
                    )
                    .send()
                    .unwrap();
            }
        });

        system!(
        "active_animation_sync",
        world,
        &Compose,
        ?&ConnectionId,
        &mut ActiveAnimation,
        )
        .kind(id::<flecs::pipeline::OnStore>())
        .each_iter(move |it, row, (compose, connection_id, animation)| {
            let io = connection_id.copied();

            let entity = it.entity(row);

            let entity_id = entity.minecraft_id();

            for pkt in animation.packets(entity_id) {
                compose
                    .broadcast_channel(
                        Clientbound::new(PacketId::Animate.to_raw(), &pkt),
                        entity.into(),
                    )
                    .exclude(io)
                    .send()
                    .unwrap();
            }

            animation.clear();
        });

        // What ever you do DO NOT!!! I REPEAT DO NOT SET VELOCITY ANYWHERE
        // IF YOU WANT TO APPLY VELOCITY SEND 1 VELOCITY PAKCET WHEN NEEDED LOOK in events/tag/src/module/attack.rs
        system!(
            "sync_player_entity",
            world,
            &Compose,
            &mut Events,
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
        .kind(id::<flecs::pipeline::PreStore>())
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
                let world = it.world();
                let entity = it.entity(row);
                let entity_id = entity.minecraft_id();

                if let Some(pending_teleport) = pending_teleport {
                    if pending_teleport.ttl == 0 {
                        entity.set::<PendingTeleportation>(PendingTeleportation::new(
                            pending_teleport.destination,
                        ));
                    }
                    pending_teleport.ttl -= 1;
                } else {
                    let position_delta = **position - tracking.last_tick_position;
                    let needs_teleport = position_delta.abs().max_element() >= 8.0;
                    let changed_position = **position != tracking.last_tick_position;

                    let look_changed = (**yaw - **prev_yaw).abs() >= 0.01
                        || (**pitch - **prev_pitch).abs() >= 0.01;

                    let mut bundle = DataBundle::new(compose);

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

                        let delta = encode_delta(tracking.last_tick_position, **position);

                        if changed_position && !needs_teleport && look_changed {
                            let packet = MoveEntityPosRot {
                                entity_id,
                                xa: delta[0],
                                ya: delta[1],
                                za: delta[2],
                                y_rot: pack_degrees(**yaw),
                                x_rot: pack_degrees(**pitch),
                                on_ground: grounded,
                            };

                            bundle
                                .add_packet(Clientbound::new(
                                    PacketId::MoveEntityPosRot.to_raw(),
                                    &packet,
                                ))
                                .unwrap();
                        } else {
                            if changed_position && !needs_teleport {
                                let packet = MoveEntityPos {
                                    entity_id,
                                    xa: delta[0],
                                    ya: delta[1],
                                    za: delta[2],
                                    on_ground: grounded,
                                };

                                bundle
                                    .add_packet(Clientbound::new(
                                        PacketId::MoveEntityPos.to_raw(),
                                        &packet,
                                    ))
                                    .unwrap();
                            }

                            if look_changed {
                                let packet = MoveEntityRot {
                                    entity_id,
                                    y_rot: pack_degrees(**yaw),
                                    x_rot: pack_degrees(**pitch),
                                    on_ground: grounded,
                                };

                                bundle
                                    .add_packet(Clientbound::new(
                                        PacketId::MoveEntityRot.to_raw(),
                                        &packet,
                                    ))
                                    .unwrap();
                            }
                            let packet = RotateHead {
                                entity_id,
                                y_head_rot: pack_degrees(**yaw),
                            };

                            bundle
                                .add_packet(Clientbound::new(
                                    PacketId::RotateHead.to_raw(),
                                    &packet,
                                ))
                                .unwrap();
                        }

                        if needs_teleport {
                            // `ClientboundEntityPositionSyncPacket`, not
                            // `TeleportEntity`: since 1.21.2 the latter carries
                            // a relative-movement bitmask and is what a client
                            // acknowledges for its *own* position, while this
                            // is what `ServerEntity` sends to observers.
                            let packet = EntityPositionSync {
                                id: entity_id,
                                values: PositionMoveRotation {
                                    position: ProtoVec3 {
                                        x: f64::from(position.x),
                                        y: f64::from(position.y),
                                        z: f64::from(position.z),
                                    },
                                    delta_movement: ProtoVec3 {
                                        x: f64::from(velocity.0.x),
                                        y: f64::from(velocity.0.y),
                                        z: f64::from(velocity.0.z),
                                    },
                                    y_rot: **yaw,
                                    x_rot: **pitch,
                                },
                                on_ground: grounded,
                            };

                            bundle
                                .add_packet(Clientbound::new(
                                    PacketId::EntityPositionSync.to_raw(),
                                    &packet,
                                ))
                                .unwrap();
                        }
                    });

                    if velocity.0 != Vec3::ZERO {
                        let packet = SetEntityMotion {
                            id: entity_id,
                            // Blocks per tick, which is what `Velocity` already
                            // holds; `Vec3.LP_STREAM_CODEC` quantises it rather
                            // than taking the 1/8000-block shorts 1.20.1 did.
                            movement: ProtoVec3 {
                                x: f64::from(velocity.0.x),
                                y: f64::from(velocity.0.y),
                                z: f64::from(velocity.0.z),
                            },
                        };

                        bundle
                            .add_packet(Clientbound::new(
                                PacketId::SetEntityMotion.to_raw(),
                                &packet,
                            ))
                            .unwrap();
                        velocity.0 = Vec3::ZERO;
                    }

                    bundle.broadcast_channel(entity.into()).unwrap();
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
                tracking.server_velocity.y =
                    0.08f64.mul_add(-0.980_000_019_073_486_3, tracking.server_velocity.y);
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
            ?&ConnectionId,
            ?&mut Yaw,
            ?&mut Pitch
        )
        .kind(id::<flecs::pipeline::OnUpdate>())
        .with_enum_wildcard::<EntityKind>()
        .each_iter(
            |it, row, (position, velocity, owner, connection_id, yaw, pitch)| {
                if let Some(_connection_id) = connection_id {
                    return;
                }

                let world = it.world();
                let arrow_entity = it.entity(row);
                // Prefer the per-instance `ProjectileMotion` that
                // `seed_projectile_motion` puts on every simulated kind, so an
                // ability can override one projectile's gravity or drag; fall
                // back to the kind's vanilla default for anything that has a
                // kind but no component yet.
                let motion = arrow_entity
                    .try_get::<&ProjectileMotion>(|motion| *motion)
                    .or_else(|| {
                        arrow_entity
                            .try_get::<&EntityKind>(|kind| kind.projectile_motion())
                            .flatten()
                    });

                if velocity.0 != Vec3::ZERO {
                    let center = **position;

                    // The ray *is* this tick's travel: `Velocity` is blocks per
                    // tick, so the segment runs from here to here-plus-velocity
                    // and `t == 1` is the far end of it. It used to be scaled by
                    // the velocity's own length as well, which asked about a
                    // segment `|v|` times too long, and against an unbounded
                    // block scan that meant an arrow stopped dead as soon as
                    // anything was ahead of it on its heading rather than when
                    // it got there.
                    let ray = geometry::ray::Ray::new(center, velocity.0);

                    let Some(collision) = get_first_collision(ray, &world, Some(owner.entity))
                    else {
                        // Vanilla's own integration, per kind, rather than one
                        // rate for everything: an arrow and a snowball disagree
                        // about their constants *and* about the order of the three
                        // statements. `crates/hyperion/tests/differential.rs`
                        // holds this against a recording of the real server.
                        if let Some(motion) = motion {
                            // A projectile points where it is going, re-aimed off
                            // its velocity every tick. Without this an arrow keeps
                            // its launch orientation for its whole flight: the arc
                            // is right but it renders frozen at its loosed angle
                            // rather than nosing over as it falls. Vanilla reads
                            // the velocity at the moment its own tick calls
                            // `updateRotation`, and that moment differs by
                            // integrator: `AbstractArrow.tick` aims from the
                            // velocity it entered the tick with, before the move
                            // and decay, while `ThrowableProjectile.tick` applies
                            // gravity and drag first and aims from the result. So
                            // the arrow is aimed before the step and the thrown
                            // kind after it.
                            // The rotation easing still runs (it keeps the
                            // stored facing correct for anything server-side
                            // that reads it) but is no longer sent: the client
                            // re-derives an arrow's heading from its velocity.
                            let _rotation = match motion.order {
                                MotionOrder::MoveThenDecay => {
                                    let rotation = aim_along(yaw, pitch, velocity.0);
                                    motion.step(position, &mut velocity.0);
                                    rotation
                                }
                                MotionOrder::DecayThenMove => {
                                    motion.step(position, &mut velocity.0);
                                    aim_along(yaw, pitch, velocity.0)
                                }
                            };

                            // Tell every client watching this arrow how fast it
                            // is going, every tick, and let the client dead-reckon
                            // the position from it. The launch `add_entity`
                            // already seeded the velocity; this keeps the client's
                            // prediction exact without the per-tick teleport that
                            // an absolute position sync would be.
                            let id = arrow_entity.minecraft_id();
                            let vel = velocity.0;
                            world.get::<&Compose>(|compose| {
                                broadcast_projectile_velocity(
                                    compose,
                                    arrow_entity.into(),
                                    id,
                                    vel,
                                );
                            });
                        }
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
            },
        );

        // The other half of `update_projectile_positions`' broadcast, for the
        // projectiles that carry no `Owner` because a game module integrates
        // their own flight. The marker, not the ownership, is what says "keep
        // telling clients where this is": a smash projectile advances its own
        // position and only wants the send. `OnStore`, after every integrator
        // has moved the entity this tick, and gated on `Channel` so the send
        // has subscribers to reach.
        system!("broadcast_marked_projectiles", world, &Compose, &Velocity,)
            .with(id::<BroadcastProjectile>())
            .with(id::<Channel>())
            .kind(id::<flecs::pipeline::OnStore>())
            .each_iter(|it, row, (compose, velocity)| {
                let entity = it.entity(row);
                broadcast_projectile_velocity(
                    compose,
                    entity.into(),
                    entity.minecraft_id(),
                    velocity.0,
                );
            });

        track_previous::<Position>(world);
        track_previous::<Yaw>(world);
        track_previous::<Pitch>(world);
    }
}
