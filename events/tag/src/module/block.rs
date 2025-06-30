use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use hyperion::{
    BlockKind, chat,
    net::{Compose, ConnectionId, agnostic},
    simulation::{
        Xp,
        blocks::{Blocks, EntityAndSequence},
        event,
    },
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
use tracing::error;

use crate::{MainBlockCount, OreVeins};

const TOTAL_DESTRUCTION_TIME: Duration = Duration::from_secs(30);

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

#[derive(Default, Resource)]
pub struct PendingDestruction {
    pub destroy_at: Scheduled<Instant, DestroyValue>,
    pub set_level_at: Scheduled<Instant, SetLevel>,
}

fn handle_pending_air(
    mut pending_air: ResMut<'_, PendingDestruction>,
    mut blocks: ResMut<'_, Blocks>,
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (&mut PlayerInventory, &mut MainBlockCount)>,
) {
    let now = Instant::now();
    for SetLevel {
        position,
        sequence,
        stage,
    } in pending_air.set_level_at.pop_until(&now)
    {
        let packet = play::BlockBreakingProgressS2c {
            entity_id: VarInt(sequence),
            position: BlockPos::new(position.x, position.y, position.z),
            destroy_stage: stage,
        };
        compose.broadcast(&packet).send().unwrap();

        let center_block = position.as_dvec3() + DVec3::splat(0.5);
        let sound = agnostic::sound(
            ident!("minecraft:block.stone.break"),
            center_block.as_vec3(),
        )
        .volume(0.35)
        .pitch(f32::from(stage).mul_add(0.1, 1.0))
        .build();

        compose.broadcast(&sound).send().unwrap();
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

        compose.broadcast(&particle_packet).send().unwrap();

        let sound = agnostic::sound(
            ident!("minecraft:entity.zombie.break_wooden_door"),
            center_block.as_vec3(),
        )
        .volume(1.0)
        .pitch(0.8)
        .seed(fastrand::i64(..))
        .build();

        compose.broadcast(&sound).send().unwrap();

        let (mut inventory, mut main_block_count) = match query.get_mut(destroy.from) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle pending air: query failed: {e}");
                continue;
            }
        };

        let stack = &mut inventory
            .get_hand_slot_mut(inventory::BLOCK_SLOT)
            .unwrap()
            .stack;

        stack.count = stack.count.saturating_add(1);
        **main_block_count = main_block_count.saturating_add(1);

        blocks.set_block(destroy.position, BlockState::AIR).unwrap();
    }
}

fn handle_destroyed_blocks(
    mut events: EventReader<'_, '_, event::DestroyBlock>,
    compose: Res<'_, Compose>,
    ore_veins: Res<'_, OreVeins>,
    mut blocks: ResMut<'_, Blocks>,
    mut query: Query<'_, '_, (&ConnectionId, &mut Xp)>,
) {
    for event in events.read() {
        blocks.to_confirm.push(EntityAndSequence {
            entity: event.from,
            sequence: event.sequence,
        });

        let (&connection_id, mut xp) = match query.get_mut(event.from) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle destroyed blocks: query failed: {e}");
                continue;
            }
        };

        if !ore_veins.ores.contains(&event.position) {
            let current = blocks.get_block(event.position).unwrap();

            // make sure the player knows the block was placed back
            let pkt = play::BlockUpdateS2c {
                position: BlockPos::new(event.position.x, event.position.y, event.position.z),
                block_id: current,
            };

            compose.unicast(&pkt, connection_id).unwrap();

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

            compose.unicast(&pkt, connection_id).unwrap();

            continue;
        }

        // replace with stone
        let Ok(..) = blocks.set_block(event.position, BlockState::STONE) else {
            continue;
        };

        **xp = xp.saturating_add(xp_amount);

        // Create a message about the broken block
        let msg = format!("{xp_amount}xp");

        let pkt = play::GameMessageS2c {
            chat: msg.into_cow_text(),
            overlay: true,
        };

        // Send the message to the player
        compose.unicast(&pkt, connection_id).unwrap();

        let position = event.position;

        let sound = agnostic::sound(
            ident!("minecraft:block.note_block.harp"),
            position.as_vec3() + Vec3::splat(0.5),
        )
        .volume(1.0)
        .pitch(1.0)
        .build();

        compose.unicast(&sound, connection_id).unwrap();
    }
}

fn handle_placed_blocks(
    mut events: EventReader<'_, '_, event::PlaceBlock>,
    mut pending_air: ResMut<'_, PendingDestruction>,
    mut blocks: ResMut<'_, Blocks>,
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (&mut MainBlockCount, &ConnectionId)>,
) {
    for event::PlaceBlock {
        position,
        block,
        from,
        sequence,
    } in events.read()
    {
        let (mut main_block_count, &connection_id) = match query.get_mut(*from) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle placed blocks: query failed: {e}");
                continue;
            }
        };

        if block.collision_shapes().is_empty() {
            blocks
                .to_confirm
                .push(EntityAndSequence::new(*from, *sequence));

            // so we send update to player

            let msg = chat!("§cYou can't place this block");

            compose.unicast(&msg, connection_id).unwrap();

            continue;
        }

        blocks.set_block(*position, *block).unwrap();

        // TODO: Removing one block from the inventory should be done in the inventory system
        **main_block_count = (**main_block_count - 1).max(0);

        let destroy = DestroyValue {
            position: *position,
            from: *from,
        };

        pending_air
            .destroy_at
            .schedule(Instant::now() + TOTAL_DESTRUCTION_TIME, destroy);

        {
            // Schedule destruction stages 0 through 9
            for stage in 0_u8..=10 {
                // 10 represents no animation
                let delay = TOTAL_DESTRUCTION_TIME / 10 * u32::from(stage);
                pending_air
                    .set_level_at
                    .schedule(Instant::now() + delay, SetLevel::new(*position, stage));
            }
        }
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
        app.insert_resource(PendingDestruction::default());
        app.add_systems(
            FixedUpdate,
            (
                handle_pending_air,
                handle_destroyed_blocks,
                handle_placed_blocks,
                handle_toggled_doors,
            ),
        );
    }
}
