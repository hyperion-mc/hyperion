use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use flecs_ecs::{
    core::{Entity, EntityViewGet, QueryBuilderImpl, SystemAPI, TableIter, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    BlockKind, chat,
    net::{Compose, ConnectionId, agnostic},
    simulation::{
        Xp,
        blocks::{Blocks, EntityAndSequence},
        event,
    },
    storage::EventQueue,
    valence_protocol::{
        BlockPos, BlockState, Particle, VarInt,
        block::{PropName, PropValue},
        ident,
        math::{DVec3, IVec3, Vec3},
        packets::play,
        text::IntoText,
    },
};
use hyperion_inventory::PlayerInventory;
use hyperion_rank_tree::inventory;
use hyperion_scheduled::Scheduled;
use tracing::{error, info_span};

use crate::{MainBlockCount, OreVeins};

#[derive(Component)]
pub struct BlockModule;

pub struct SetLevel {
    pub position: IVec3,
    pub sequence: i32,
    pub stage: u8,
}

impl SetLevel {
    pub fn new(position: IVec3, stage: u8) -> Self {
        Self {
            position,
            sequence: fastrand::i32(..),
            stage,
        }
    }
}

pub struct DestroyValue {
    pub position: IVec3,
    pub from: Entity,
}

#[derive(Default, Component)]
pub struct PendingDestruction {
    pub destroy_at: Scheduled<Instant, DestroyValue>,
    pub set_level_at: Scheduled<Instant, SetLevel>,
}

impl Module for BlockModule {
    #[allow(clippy::excessive_nesting)]
    fn module(world: &World) {
        const TOTAL_DESTRUCTION_TIME: Duration = Duration::from_secs(30);

        world.component::<PendingDestruction>();
        world.set(PendingDestruction::default());

        system!("handle_pending_air", world, &mut PendingDestruction($), &mut Blocks($), &Compose($))
            .write::<PlayerInventory>()
            .multi_threaded()
            .each_iter(
                move |it: TableIter<'_, false>,
                      _,
                      (pending_air, blocks, compose): (&mut PendingDestruction, &mut Blocks, &Compose)| {
                    let span = info_span!("handle_pending_air");
                    let _enter = span.enter();

                    let system = it.system();
                    let now = Instant::now();
                    let world = it.world();
                    for SetLevel { position, sequence, stage } in pending_air.set_level_at.pop_until(&now) {
                        let packet = play::BlockBreakingProgressS2c {
                            entity_id: VarInt(sequence),
                            position: BlockPos::new(position.x, position.y, position.z),
                            destroy_stage: stage,
                        };
                        compose.broadcast(&packet, system)
                            .send()
                            .unwrap();

                        let center_block = position.as_dvec3() + DVec3::splat(0.5);
                        let sound = agnostic::sound(
                            ident!("minecraft:block.stone.break"),
                            center_block.as_vec3(),
                        ).volume(0.35)
                            .pitch(f32::from(stage).mul_add(0.1, 1.0))
                            .build();

                        compose.broadcast(&sound, system)
                            .send()
                            .unwrap();
                    }

                    for destroy in pending_air.destroy_at.pop_until(&now) {
                        // Play particle effect for block destruction
                        let center_block = destroy.position.as_dvec3() + DVec3::splat(0.5);

                        let particle_packet = play::ParticleS2c {
                            particle: Cow::Owned(Particle::Explosion),
                            long_distance: false,
                            position: center_block,
                            offset: Vec3::default(),
                            max_speed: 0.0,
                            count: 0,
                        };

                        compose.broadcast(&particle_packet, system)
                            .send()
                            .unwrap();

                        let sound = agnostic::sound(
                            ident!("minecraft:entity.zombie.break_wooden_door"),
                            center_block.as_vec3(),
                        ).volume(1.0)
                            .pitch(0.8)
                            .seed(fastrand::i64(..))
                            .build();

                        compose.broadcast(&sound, system)
                            .send()
                            .unwrap();

                        destroy.from
                            .entity_view(world)
                            .get::<(&mut PlayerInventory, &mut MainBlockCount)>(|(inventory, main_block_count)| {
                                let stack = &mut inventory
                                    .get_hand_slot_mut(inventory::BLOCK_SLOT)
                                    .unwrap()
                                    .stack;

                                stack.count = stack.count.saturating_add(1);
                                **main_block_count = main_block_count.saturating_add(1);
                            });


                        blocks.set_block(destroy.position, BlockState::AIR).unwrap();
                    }
                },
            );

        system!("handle_destroyed_blocks", world, &mut Blocks($), &mut EventQueue<event::DestroyBlock>($), &Compose($), &OreVeins($))
            .multi_threaded()
            .each_iter(move |it: TableIter<'_, false>, _, (blocks, event_queue, compose, ore_veins): (&mut Blocks, &mut EventQueue<event::DestroyBlock>, &Compose, &OreVeins)| {
                let span = info_span!("handle_blocks");
                let _enter = span.enter();
                let system = it.system();
                let world = it.world();


                for event in event_queue.drain() {
                    blocks.to_confirm.push(EntityAndSequence {
                        entity: event.from,
                        sequence: event.sequence,
                    });

                    if !ore_veins.ores.contains(&event.position) {
                        let current = blocks.get_block(event.position).unwrap();

                        // make sure the player knows the block was placed back
                        let pkt = play::BlockUpdateS2c {
                            position: BlockPos::new(event.position.x, event.position.y, event.position.z),
                            block_id: current,
                        };

                        event.from.entity_view(world).get::<&ConnectionId>(|stream| {
                            compose.unicast(&pkt, *stream, system).unwrap();
                        });

                        continue;
                    }

                    let current = blocks.get_block(event.position).unwrap();

                    let xp_amount = match current.to_kind() {
                        BlockKind::CoalOre => 1_u16,
                        BlockKind::CopperOre => 3,
                        BlockKind::IronOre => 9,
                        BlockKind::GoldOre => 27,
                        BlockKind::EmeraldOre => 81,
                        _ => 0,
                    } * 4;

                    if xp_amount == 0 {

                        // make sure the player knows the block was placed back
                        let pkt = play::BlockUpdateS2c {
                            position: BlockPos::new(event.position.x, event.position.y, event.position.z),
                            block_id: current,
                        };

                        event.from.entity_view(world).get::<&ConnectionId>(|stream| {
                            compose.unicast(&pkt, *stream, system).unwrap();
                        });

                        continue;
                    }

                    // replace with stone
                    let Ok(..) = blocks.set_block(event.position, BlockState::STONE) else {
                        return;
                    };


                    let from = event.from;
                    let from_entity = world.entity_from_id(from);
                    from_entity.get::<(&ConnectionId, &mut Xp)>(|(&net, xp)| {
                        **xp = xp.saturating_add(xp_amount);


                        // Create a message about the broken block
                        let msg = format!("{xp_amount}xp");

                        let pkt = play::GameMessageS2c {
                            chat: msg.into_cow_text(),
                            overlay: true,
                        };

                        // Send the message to the player
                        compose.unicast(&pkt, net, system).unwrap();

                        let position = event.position;

                        let sound = agnostic::sound(
                            ident!("minecraft:block.note_block.harp"),
                            position.as_vec3() + Vec3::splat(0.5),
                        ).volume(1.0)
                            .pitch(1.0)
                            .build();

                        compose.unicast(&sound, net, system).unwrap();
                    });
                }
            });

        system!("handle_placed_blocks", world, &mut Blocks($), &mut EventQueue<event::PlaceBlock>($), &mut PendingDestruction($), &Compose($))
            .each_iter(move |it, _, (mc, event_queue, pending_air, compose): (&mut Blocks, &mut EventQueue<event::PlaceBlock>, &mut PendingDestruction, &Compose)| {
                let world = it.world();
                let span = info_span!("handle_placed_blocks");
                let _enter = span.enter();
                let system = it.system();
                for event::PlaceBlock { position, block, from, sequence } in event_queue.drain() {
                    if block.collision_shapes().is_empty() {
                        mc.to_confirm.push(EntityAndSequence::new(from, sequence));

                        from.entity_view(world).get::<(&mut PlayerInventory, &ConnectionId)>(|(_inventory, stream)| {
                            // so we send update to player

                            let msg = chat!("§cYou can't place this block");

                            compose.unicast(&msg, *stream, system).unwrap();
                        });

                        continue;
                    }

                    mc.set_block(position, block).unwrap();

                    // TODO: Removing one block from the inventory should be done in the inventory system

                    from.entity_view(world).get::<&mut MainBlockCount>(|main_block_count| {
                        **main_block_count = (**main_block_count - 1).max(0);
                    });

                    let destroy = DestroyValue {
                        position,
                        from,
                    };


                    pending_air.destroy_at.schedule(Instant::now() + TOTAL_DESTRUCTION_TIME, destroy);

                    {
                        // Schedule destruction stages 0 through 9
                        for stage in 0_u8..=10 { // 10 represents no animation
                            let delay = TOTAL_DESTRUCTION_TIME / 10 * u32::from(stage);
                            pending_air.set_level_at.schedule(
                                Instant::now() + delay,
                                SetLevel::new(position, stage),
                            );
                        }
                    }
                    mc.to_confirm.push(EntityAndSequence {
                        entity: from,
                        sequence,
                    });
                }
            });

        system!("handle_toggled_doors", world, &mut Blocks($), &mut EventQueue<event::ToggleDoor>($))
            .multi_threaded()
            .each_iter(move |_it: TableIter<'_, false>, _, (mc, event_queue): (&mut Blocks, &mut EventQueue<event::ToggleDoor>)| {
                let span = info_span!("handle_toggled_doors");
                let _enter = span.enter();
                for event in event_queue.drain() {
                    let position = event.position;

                    // The block is fetched again instead of sending the expected block state
                    // through the ToggleDoor event to avoid potential duplication bugs if the
                    // ToggleDoor event is sent, the door is broken, and the ToggleDoor event is
                    // processed
                    let Some(door) = mc.get_block(position) else { continue };
                    let Some(open) = door.get(PropName::Open) else { continue };

                    // Toggle the door state
                    let open = match open {
                        PropValue::False => PropValue::True,
                        PropValue::True => PropValue::False,
                        _ => {
                            error!("Door property 'Open' must be either 'True' or 'False'");
                            continue;
                        }
                    };

                    let door = door.set(PropName::Open, open);
                    mc.set_block(position, door).unwrap();

                    // Vertical doors (as in doors that are not trapdoors) need to have the other
                    // half of the door updated.
                    let other_half_position = match door.get(PropName::Half) {
                        Some(PropValue::Upper) => Some(position - IVec3::new(0, 1, 0)),
                        Some(PropValue::Lower) => Some(position + IVec3::new(0, 1, 0)),
                        Some(_) => {
                            error!("Door property 'Half' must be either 'Upper' or 'Lower'");
                            continue;
                        }
                        None => None
                    };

                    if let Some(other_half_position) = other_half_position {
                        let Some(other_half) = mc.get_block(other_half_position) else {
                            error!("Could not find other half of door");
                            continue;
                        };

                        let other_half = other_half.set(PropName::Open, open);
                        mc.set_block(other_half_position, other_half).unwrap();
                    }

                    mc.to_confirm.push(EntityAndSequence {
                        entity: event.from,
                        sequence: event.sequence,
                    });
                }
            });
    }
}
