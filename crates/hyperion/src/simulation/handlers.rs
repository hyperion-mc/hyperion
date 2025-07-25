use bevy::prelude::*;
use glam::DVec3;
use hyperion_inventory::PlayerInventory;
use hyperion_utils::next_lowest;
use tracing::{error, warn};
use valence_generated::{
    block::{BlockKind, BlockState, PropName},
    item::ItemKind,
};
use valence_protocol::{
    Hand, VarInt,
    packets::play::{
        GameMessageS2c, OpenWrittenBookS2c, UpdatePlayerAbilitiesC2s,
        client_command_c2s::ClientCommand, player_action_c2s::PlayerAction,
    },
};
use valence_text::IntoText;

use crate::{
    ingress,
    net::{Compose, ConnectionId},
    simulation::{
        Aabb, ConfirmBlockSequences, EntitySize, Flight, MovementTracking, PendingTeleportation,
        Pitch, Position, Yaw, aabb,
        animation::{self, ActiveAnimation},
        block_bounds,
        blocks::Blocks,
        event,
        metadata::{entity::Pose, living_entity::HandStates},
        packet::{OrderedPacketRef, play},
    },
};

#[expect(
    clippy::cognitive_complexity,
    reason = "This cannot be split into different systems because the events across the various \
              EventReaders must be processed in order. Splitting each packet handler to different \
              functions would add complexity because most parameters will still need to be passed \
              manually."
)]
fn position_and_look_updates(
    mut full_reader: EventReader<'_, '_, play::Full>,
    mut position_reader: EventReader<'_, '_, play::PositionAndOnGround>,
    mut look_reader: EventReader<'_, '_, play::LookAndOnGround>,
    mut teleport_reader: EventReader<'_, '_, play::TeleportConfirm>,
    mut queries: ParamSet<
        '_,
        '_,
        (
            Query<'_, '_, (&EntitySize, &mut MovementTracking, &mut Position, &Yaw)>,
            Query<'_, '_, (&mut Yaw, &mut Pitch)>,
            Query<'_, '_, &mut Position>,
        ),
    >,
    teleport_query: Query<'_, '_, &PendingTeleportation>,
    blocks: Res<'_, Blocks>,
    compose: Res<'_, Compose>,
    mut commands: Commands<'_, '_>,
) {
    let mut full_reader = full_reader.read().map(OrderedPacketRef::from).peekable();
    let mut position_reader = position_reader
        .read()
        .map(OrderedPacketRef::from)
        .peekable();
    let mut look_reader = look_reader.read().map(OrderedPacketRef::from).peekable();
    let mut teleport_reader = teleport_reader
        .read()
        .map(OrderedPacketRef::from)
        .peekable();
    let blocks = blocks.into_inner();
    let compose = compose.into_inner();

    loop {
        // next_lowest is used to process the packet which was sent first. It is important to
        // process position packets of different types in the order they were sent by the client
        // so the client is at the correct final position after processing all packets.
        let result = next_lowest! {
            packet in full_reader => {
                change_position_or_correct_client(
                    packet.sender(),
                    packet.connection_id(),
                    queries.p0(),
                    blocks,
                    compose,
                    &mut commands,
                    packet.position.as_vec3(),
                    packet.on_ground,
                );

                let mut query = queries.p1();
                let (mut yaw, mut pitch) = match query.get_mut(packet.sender()) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("failed to handle full packet: query failed: {e}");
                        continue;
                    }
                };

                yaw.yaw = packet.yaw;
                pitch.pitch = packet.pitch;
            },
            packet in position_reader => {
                change_position_or_correct_client(
                    packet.sender(),
                    packet.connection_id(),
                    queries.p0(),
                    blocks,
                    compose,
                    &mut commands,
                    packet.position.as_vec3(),
                    packet.on_ground,
                );
            },
            packet in look_reader => {
                let mut query = queries.p1();
                let (mut yaw, mut pitch) = match query.get_mut(packet.sender()) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("failed to handle look and on ground: query failed: {e}");
                        continue;
                    }
                };

                yaw.yaw = packet.yaw;
                pitch.pitch = packet.pitch;
            },
            packet in teleport_reader => {
                let client = packet.sender();
                let Ok(pending_teleport) = teleport_query.get(client) else {
                    warn!("failed to confirm teleportation: client is not pending teleportation, so there is nothing to confirm");
                    continue;
                };

                let pending_teleport_id = pending_teleport.teleport_id;

                if VarInt(pending_teleport_id) != packet.teleport_id {
                    // If this is reached and the client is behaving correctly, the client has been
                    // teleported again (with teleport id `pending_teleport_id`) since the initial teleport
                    // (with teleport id `packet.teleport_id`). The current teleport confirmation
                    // can be ignored; the client will need to send a new one for the newer
                    // teleport.
                    continue;
                }

                let mut query = queries.p2();
                let mut position  = match query.get_mut(client) {
                    Ok(position) => position,
                    Err(e) => {
                        error!("failed to confirm teleportation: query failed: {e}");
                        continue;
                    }
                };

                **position = pending_teleport.destination;

                commands.queue(move |world: &mut World| {
                    let Ok(mut entity) = world.get_entity_mut(client) else {
                        error!("failed to confirm teleportation: client entity has despawned");
                        return;
                    };

                    let Some(pending_teleport) = entity.get::<PendingTeleportation>() else {
                        error!(
                            "failed to confirm teleportation: client is missing PendingTeleportation \
                             component"
                        );
                        return;
                    };

                    if pending_teleport.teleport_id != pending_teleport_id {
                        // A new pending teleport must have started between the time that this
                        // command was queued and the time that this command was ran. Therefore,
                        // this should not remove the PendingTeleportation component.
                        return;
                    }

                    entity.remove::<PendingTeleportation>();
                });
            }
        };
        if result.is_none() {
            break;
        }
    }
}

fn change_position_or_correct_client(
    client: Entity,
    connection_id: ConnectionId,
    mut query: Query<'_, '_, (&EntitySize, &mut MovementTracking, &mut Position, &Yaw)>,
    blocks: &Blocks,
    compose: &Compose,
    commands: &mut Commands<'_, '_>,
    proposed: Vec3,
    on_ground: bool,
) {
    let (&size, mut tracking, mut pose, yaw) = match query.get_mut(client) {
        Ok(data) => data,
        Err(e) => {
            error!("change_position_or_correct_client failed: query failed: {e}");
            return;
        }
    };

    if let Err(e) = try_change_position(proposed, &pose, size, blocks) {
        // Send error message to player
        let msg = format!("§c{e}");
        let pkt = GameMessageS2c {
            chat: msg.into_cow_text(),
            overlay: false,
        };

        if let Err(e) = compose.unicast(&pkt, connection_id) {
            warn!("Failed to send error message to player: {e}");
        }

        commands
            .entity(client)
            .insert(PendingTeleportation::new(pose.position));
    }

    tracking.received_movement_packets = tracking.received_movement_packets.saturating_add(1);
    let y_delta = proposed.y - pose.y;

    if y_delta > 0. && tracking.was_on_ground && !on_ground {
        tracking.server_velocity.y = 0.419_999_986_886_978_15;

        if tracking.sprinting {
            let smth = yaw.yaw * 0.017_453_292;
            tracking.server_velocity += DVec3::new(
                f64::from(-smth.sin()) * 0.2,
                0.0,
                f64::from(smth.cos()) * 0.2,
            );
        }
    }

    **pose = proposed;
}

/// Returns true if the position was changed, false if it was not.
///
/// Movement validity rules:
/// ```text
///   From  |   To    | Allowed
/// --------|---------|--------
/// in  🧱  | in  🧱  |   ✅
/// in  🧱  | out 🌫️  |   ✅
/// out 🌫️  | in  🧱  |   ❌
/// out 🌫️  | out 🌫️  |   ✅
/// ```
/// Only denies movement if starting outside a block and moving into a block.
/// This prevents players from glitching into blocks while allowing them to move out.
fn try_change_position(
    proposed: Vec3,
    position: &Position,
    size: EntitySize,
    blocks: &Blocks,
) -> anyhow::Result<()> {
    // Only check collision if we're starting outside a block
    if !has_block_collision(position, size, blocks) && has_block_collision(&proposed, size, blocks)
    {
        return Err(anyhow::anyhow!("Cannot move into solid blocks"));
    }

    Ok(())
}

#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn is_grounded(position: &Vec3, blocks: &Blocks) -> bool {
    // Calculate the block position by flooring the x and z coordinates
    let block_x = position.x as i32;
    let block_y = (position.y.ceil() - 1.0) as i32; // Check the block directly below
    let block_z = position.z as i32;

    // Check if the block at the calculated position is not air
    !blocks
        .get_block(IVec3::new(block_x, block_y, block_z))
        .is_some_and(BlockState::is_air)
}

fn has_block_collision(position: &Vec3, size: EntitySize, blocks: &Blocks) -> bool {
    use std::ops::ControlFlow;

    let (min, max) = block_bounds(*position, size);
    let shrunk = aabb(*position, size).shrink(0.01);

    let res = blocks.get_blocks(min, max, |pos, block| {
        let pos = Vec3::new(pos.x as f32, pos.y as f32, pos.z as f32);

        for aabb in block.collision_shapes() {
            let aabb = Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3());
            let aabb = aabb.move_by(pos);

            if shrunk.collides(&aabb) {
                return ControlFlow::Break(false);
            }
        }

        ControlFlow::Continue(())
    });

    res.is_break()
}

fn hand_swing(
    mut packets: EventReader<'_, '_, play::HandSwing>,
    mut query: Query<'_, '_, &mut ActiveAnimation>,
) {
    for packet in packets.read() {
        let mut animation = match query.get_mut(packet.sender()) {
            Ok(animation) => animation,
            Err(e) => {
                error!("failed to handle hand swing: query failed: {e}");
                continue;
            }
        };

        match packet.hand {
            Hand::Main => {
                animation.push(animation::Kind::SwingMainArm);
            }
            Hand::Off => {
                animation.push(animation::Kind::SwingOffHand);
            }
        }
    }
}

// i.e., shooting a bow, digging a block, etc
fn player_action(
    mut packets: EventReader<'_, '_, play::PlayerAction>,
    mut start_destroy_writer: EventWriter<'_, event::StartDestroyBlock>,
    mut stop_destroy_writer: EventWriter<'_, event::DestroyBlock>,
    mut release_writer: EventWriter<'_, event::ReleaseUseItem>,
    mut commands: Commands<'_, '_>,
) {
    for packet in packets.read() {
        let sequence = packet.sequence.0;
        let position = IVec3::new(packet.position.x, packet.position.y, packet.position.z);

        match packet.action {
            PlayerAction::StartDestroyBlock => {
                let event = event::StartDestroyBlock {
                    position,
                    from: packet.sender(),
                    sequence,
                };
                start_destroy_writer.write(event);
            }
            PlayerAction::StopDestroyBlock => {
                let event = event::DestroyBlock {
                    position,
                    from: packet.sender(),
                    sequence,
                };

                stop_destroy_writer.write(event);
            }
            PlayerAction::ReleaseUseItem => {
                let event = event::ReleaseUseItem {
                    from: packet.sender(),
                };

                commands.entity(packet.sender()).insert(HandStates::new(0));

                release_writer.write(event);
            }
            action => error!("failed to handle player action: unimplemented {action:?}"),
        }

        // todo: implement
    }
}

// for sneaking/crouching/etc
fn client_command(
    mut packets: EventReader<'_, '_, play::ClientCommand>,
    mut query: Query<'_, '_, (&mut Pose, &mut EntitySize, &mut MovementTracking)>,
) {
    for packet in packets.read() {
        let (mut pose, mut size, mut tracking) = match query.get_mut(packet.sender()) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to handle client command: query failed: {e}");
                continue;
            }
        };

        match packet.action {
            ClientCommand::StartSneaking => {
                *pose = Pose::Sneaking;
                size.height = 1.5;
            }
            ClientCommand::StopSneaking | ClientCommand::LeaveBed => {
                *pose = Pose::Standing;
                size.height = 1.8;
            }
            ClientCommand::StartSprinting => {
                tracking.sprinting = true;
            }
            ClientCommand::StopSprinting => {
                tracking.sprinting = false;
            }
            ClientCommand::StartJumpWithHorse
            | ClientCommand::StopJumpWithHorse
            | ClientCommand::OpenHorseInventory
            | ClientCommand::StartFlyingWithElytra => {}
        }
    }
}

/// Handles player interaction with items in hand
///
/// Common uses:
/// - Starting to wind up a bow for shooting arrows
/// - Using consumable items like food or potions
/// - Throwing items like snowballs or ender pearls
/// - Using tools/items with special right-click actions (e.g. fishing rods, shields)
/// - Activating items with duration effects (e.g. chorus fruit teleport)
fn player_interact_item(
    mut packets: EventReader<'_, '_, play::PlayerInteractItem>,
    compose: Res<'_, Compose>,
    query: Query<'_, '_, &PlayerInventory>,
    mut interact_event_writer: EventWriter<'_, event::InteractEvent>,
    mut item_interact_writer: EventWriter<'_, event::ItemInteract>,
) {
    for packet in packets.read() {
        let inventory = match query.get(packet.sender()) {
            Ok(inventory) => inventory,
            Err(e) => {
                error!("failed to process player interact item: query failed: {e}");
                continue;
            }
        };

        let event = event::InteractEvent {
            client: packet.sender(),
            hand: packet.hand,
            sequence: packet.sequence.0,
        };

        let cursor = &inventory.get_cursor().stack;

        if !cursor.is_empty() {
            let event = event::ItemInteract {
                entity: packet.sender(),
                hand: packet.hand,
                sequence: packet.sequence.0,
            };
            if cursor.item == ItemKind::WrittenBook {
                compose
                    .unicast(
                        &OpenWrittenBookS2c { hand: packet.hand },
                        packet.connection_id(),
                    )
                    .unwrap();
            }
            item_interact_writer.write(event);
        }

        interact_event_writer.write(event);
    }
}

fn player_interact_block(
    mut packets: EventReader<'_, '_, play::PlayerInteractBlock>,
    mut query: Query<
        '_,
        '_,
        (
            &mut ConfirmBlockSequences,
            &PlayerInventory,
            &Position,
            &EntitySize,
        ),
    >,
    blocks: Res<'_, Blocks>,
    mut toggle_door_writer: EventWriter<'_, event::ToggleDoor>,
    mut place_block_writer: EventWriter<'_, event::PlaceBlock>,
) {
    for packet in packets.read() {
        // PlayerInteractBlock contains:
        // - hand: Hand (enum: MainHand or OffHand)
        // - position: BlockPos (x, y, z coordinates of the block)
        // - face: Direction (enum: Down, Up, North, South, West, East)
        // - cursor_position: Vec3 (x, y, z coordinates of cursor on the block face)
        // - inside_block: bool (whether the player's head is inside a block)
        // - sequence: VarInt (sequence number for this interaction)

        let (mut confirm_block_sequences, inventory, client_position, size) =
            match query.get_mut(packet.sender()) {
                Ok(data) => data,
                Err(e) => {
                    error!("failed to handle player interact block: query failed: {e}");
                    continue;
                }
            };

        confirm_block_sequences.push(packet.sequence.0);

        let interacted_block_pos = packet.position;
        let interacted_block_pos_vec = IVec3::new(
            interacted_block_pos.x,
            interacted_block_pos.y,
            interacted_block_pos.z,
        );

        let Some(interacted_block) = blocks.get_block(interacted_block_pos_vec) else {
            continue;
        };

        if interacted_block.get(PropName::Open).is_some() {
            // Toggle the open state of a door
            // todo: place block instead of toggling door if the player is crouching and holding a
            // block

            toggle_door_writer.write(event::ToggleDoor {
                position: interacted_block_pos_vec,
                from: packet.sender(),
                sequence: packet.sequence.0,
            });
        } else {
            // Attempt to place a block

            let held = &inventory.get_cursor().stack;

            if held.is_empty() {
                continue;
            }

            let kind = held.item;

            let Some(block_kind) = BlockKind::from_item_kind(kind) else {
                warn!("invalid item kind to place: {kind:?}");
                continue;
            };

            let block_state = BlockState::from_kind(block_kind);

            let position = interacted_block_pos.get_in_direction(packet.face);
            let position = IVec3::new(position.x, position.y, position.z);

            let position_dvec3 = position.as_vec3();

            // todo(hack): technically players can do some crazy position stuff to abuse this probably
            let player_aabb = aabb(**client_position, *size);

            let collides_player = block_state
                .collision_shapes()
                .map(|aabb| {
                    Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3()).move_by(position_dvec3)
                })
                .any(|block_aabb| Aabb::overlap(&block_aabb, &player_aabb).is_some());

            if collides_player {
                continue;
            }

            place_block_writer.write(event::PlaceBlock {
                position,
                from: packet.sender(),
                sequence: packet.sequence.0,
                block: block_state,
            });
        }
    }
}

fn creative_inventory_action(
    mut packets: EventReader<'_, '_, play::CreativeInventoryAction>,
    mut query: Query<'_, '_, &mut PlayerInventory>,
) {
    for packet in packets.read() {
        // TODO: Verify that the player is in creative mode

        let Ok(slot) = u16::try_from(packet.slot) else {
            warn!("invalid slot {}", packet.slot);
            continue;
        };

        let mut inventory = match query.get_mut(packet.sender()) {
            Ok(inventory) => inventory,
            Err(e) => {
                error!("failed to handle creative inventory action: query failed: {e}");
                continue;
            }
        };

        if let Err(e) = inventory.set(slot, packet.clicked_item.clone()) {
            error!("failed to handle creative inventory action: inventory set failed: {e}");
        }
    }
}

fn player_abilities(
    mut packets: EventReader<'_, '_, play::UpdatePlayerAbilities>,
    mut query: Query<'_, '_, &mut Flight>,
) {
    for packet in packets.read() {
        let mut flight = match query.get_mut(packet.sender()) {
            Ok(flight) => flight,
            Err(e) => {
                error!("player abilities failed: query failed: {e}");
                continue;
            }
        };

        match **packet {
            UpdatePlayerAbilitiesC2s::StopFlying => flight.is_flying = false,
            UpdatePlayerAbilitiesC2s::StartFlying => flight.is_flying = flight.allow,
        }
    }
}

pub struct HandlersPlugin;

impl Plugin for HandlersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                position_and_look_updates,
                hand_swing,
                player_action,
                client_command,
                player_interact_item,
                player_interact_block,
                creative_inventory_action,
                player_abilities,
            )
                .after(ingress::decode::play),
        );
    }
}
