use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    chat,
    net::{Compose, ConnectionId},
    simulation::{
        blocks::{Blocks, EntityAndSequence},
        event,
    },
    storage::EventQueue,
    valence_protocol::{
        BlockPos,
        block::{PropName, PropValue},
        math::IVec3,
        packets::play,
    },
};
use tracing::{error, info_span};

#[derive(Component)]
pub struct BlockModule;

impl Module for BlockModule {
    fn module(world: &World) {
        system!(
            "handle_destroyed_blocks",
            world,
            &mut Blocks,
            &mut EventQueue<event::DestroyBlock>,
            &Compose,
        )
        .each_iter(|it, _, (blocks, event_queue, compose)| {
            let span = info_span!("handle_destroyed_blocks");
            let _enter = span.enter();
            let world = it.world();

            for event in event_queue.drain() {
                blocks.to_confirm.push(EntityAndSequence {
                    entity: event.from,
                    sequence: event.sequence,
                });

                let current = blocks.get_block(event.position).unwrap();

                // make sure the player knows the block was placed back
                let pkt = play::BlockUpdateS2c {
                    position: BlockPos::new(event.position.x, event.position.y, event.position.z),
                    block_id: current,
                };

                event
                    .from
                    .entity_view(world)
                    .get::<&ConnectionId>(|connection_id| {
                        compose.unicast(&pkt, *connection_id).unwrap();
                    });
            }
        });

        system!(
            "handle_placed_blocks",
            world,
            &mut Blocks,
            &mut EventQueue<event::PlaceBlock>,
            &Compose,
        )
        .each_iter(|it, _, (blocks, event_queue, compose)| {
            let span = info_span!("handle_placed_blocks");
            let _enter = span.enter();
            let world = it.world();

            for event::PlaceBlock {
                position,
                block,
                from,
                sequence,
            } in event_queue.drain()
            {
                if block.collision_shapes().is_empty() {
                    blocks
                        .to_confirm
                        .push(EntityAndSequence::new(from, sequence));

                    // so we send update to player
                    let msg = chat!("§cYou can't place this block");

                    from.entity_view(world)
                        .get::<&ConnectionId>(|connection_id| {
                            compose.unicast(&msg, *connection_id).unwrap();
                        });

                    continue;
                }

                blocks.set_block(position, block).unwrap();

                blocks.to_confirm.push(EntityAndSequence {
                    entity: from,
                    sequence,
                });
            }
        });

        system!(
            "handle_toggled_doors",
            world,
            &mut Blocks,
            &mut EventQueue<event::ToggleDoor>,
        )
        .each(|(blocks, event_queue)| {
            let span = info_span!("handle_toggled_doors");
            let _enter = span.enter();

            for event in event_queue.drain() {
                let position = event.position;

                // The block is fetched again instead of sending the expected block state
                // through the ToggleDoor event to avoid potential duplication bugs if the
                // ToggleDoor event is sent, the door is broken, and the ToggleDoor event is
                // processed
                let Some(door) = blocks.get_block(position) else {
                    continue;
                };
                let Some(open) = door.get(PropName::Open) else {
                    continue;
                };

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
                blocks.set_block(position, door).unwrap();

                // Vertical doors (as in doors that are not trapdoors) need to have the other
                // half of the door updated.
                let other_half_position = match door.get(PropName::Half) {
                    Some(PropValue::Upper) => Some(position - IVec3::new(0, 1, 0)),
                    Some(PropValue::Lower) => Some(position + IVec3::new(0, 1, 0)),
                    Some(_) => {
                        error!("Door property 'Half' must be either 'Upper' or 'Lower'");
                        continue;
                    }
                    None => None,
                };

                if let Some(other_half_position) = other_half_position {
                    let Some(other_half) = blocks.get_block(other_half_position) else {
                        error!("Could not find other half of door");
                        continue;
                    };

                    let other_half = other_half.set(PropName::Open, open);
                    blocks.set_block(other_half_position, other_half).unwrap();
                }

                blocks.to_confirm.push(EntityAndSequence {
                    entity: event.from,
                    sequence: event.sequence,
                });
            }
        });
    }
}
