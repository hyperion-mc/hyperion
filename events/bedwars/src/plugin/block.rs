use bevy::prelude::*;
use hyperion::{
    chat,
    net::{Compose, ConnectionId},
    simulation::{
        blocks::{Blocks, EntityAndSequence},
        event,
    },
    valence_protocol::{
        BlockPos,
        block::{PropName, PropValue},
        math::IVec3,
        packets::play,
    },
};
use tracing::error;

fn handle_destroyed_blocks(
    mut events: EventReader<'_, '_, event::DestroyBlock>,
    compose: Res<'_, Compose>,
    mut blocks: ResMut<'_, Blocks>,
    query: Query<'_, '_, &ConnectionId>,
) {
    for event in events.read() {
        blocks.to_confirm.push(EntityAndSequence {
            entity: event.from,
            sequence: event.sequence,
        });

        let &connection_id = match query.get(event.from) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle destroyed blocks: query failed: {e}");
                continue;
            }
        };

        let current = blocks.get_block(event.position).unwrap();

        // make sure the player knows the block was placed back
        let pkt = play::BlockUpdateS2c {
            position: BlockPos::new(event.position.x, event.position.y, event.position.z),
            block_id: current,
        };

        compose.unicast(&pkt, connection_id).unwrap();
    }
}

fn handle_placed_blocks(
    mut events: EventReader<'_, '_, event::PlaceBlock>,
    mut blocks: ResMut<'_, Blocks>,
    compose: Res<'_, Compose>,
    query: Query<'_, '_, &ConnectionId>,
) {
    for event::PlaceBlock {
        position,
        block,
        from,
        sequence,
    } in events.read()
    {
        let &connection_id = match query.get(*from) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle placed blocks: query failed: {e}");
                continue;
            }
        };

        if block.collision_shapes().len() == 0 {
            blocks
                .to_confirm
                .push(EntityAndSequence::new(*from, *sequence));

            // so we send update to player

            let msg = chat!("§cYou can't place this block");

            compose.unicast(&msg, connection_id).unwrap();

            continue;
        }

        blocks.set_block(*position, *block).unwrap();

        blocks.to_confirm.push(EntityAndSequence {
            entity: *from,
            sequence: *sequence,
        });
    }
}

fn handle_toggled_doors(
    mut events: EventReader<'_, '_, event::ToggleDoor>,
    mut blocks: ResMut<'_, Blocks>,
) {
    for event in events.read() {
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
}

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                handle_destroyed_blocks,
                handle_placed_blocks,
                handle_toggled_doors,
            ),
        );
    }
}
