use bevy::prelude::*;
use hyperion_inventory::PlayerInventory;
use tracing::{error, info};
use valence_generated::item::ItemKind;
use valence_protocol::packets::play::OpenWrittenBookS2c;

// use super::{
//     ConfirmBlockSequences, EntitySize, Flight, MovementTracking, PendingTeleportation, Position,
//     animation::{self, ActiveAnimation},
//     block_bounds,
//     blocks::Blocks,
//     event::ClientStatusEvent,
//     inventory::{handle_click_slot, handle_update_selected_slot},
// };
use crate::{
    net::Compose,
    simulation::{
        Name,
        // metadata::{entity::Pose, living_entity::HandStates},
        event,
        packet::play,
    },
};

// fn full(
//     &play::FullC2s {
//         position,
//         yaw,
//         pitch,
//         on_ground,
//     }: &play::FullC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     // check to see if the player is moving too fast
//     // if they are, ignore the packet
//
//     let position = position.as_vec3();
//     change_position_or_correct_client(query, position, on_ground);
//
//     query.yaw.yaw = yaw;
//     query.pitch.pitch = pitch;
//
//     Ok(())
// }
//
// // #[instrument(skip_all)]
// fn change_position_or_correct_client(
//     query: &mut PacketSwitchQuery<'_>,
//     proposed: Vec3,
//     on_ground: bool,
// ) {
//     let pose = &mut *query.position;
//
//     if let Err(e) = try_change_position(proposed, pose, *query.size, query.blocks) {
//         // Send error message to player
//         let msg = format!("§c{e}");
//         let pkt = play::GameMessageS2c {
//             chat: msg.into_cow_text(),
//             overlay: false,
//         };
//
//         if let Err(e) = query.compose.unicast(&pkt, query.io_ref, query.system) {
//             warn!("Failed to send error message to player: {e}");
//         }
//
//         query
//             .id
//             .entity_view(query.world)
//             .set(PendingTeleportation::new(pose.position));
//     }
//     query.view.get::<&mut MovementTracking>(|tracking| {
//         tracking.received_movement_packets = tracking.received_movement_packets.saturating_add(1);
//         let y_delta = proposed.y - pose.y;
//
//         if y_delta > 0. && tracking.was_on_ground && !on_ground {
//             tracking.server_velocity.y = 0.419_999_986_886_978_15;
//
//             if tracking.sprinting {
//                 let smth = query.yaw.yaw * 0.017_453_292;
//                 tracking.server_velocity += DVec3::new(
//                     f64::from(-smth.sin()) * 0.2,
//                     0.0,
//                     f64::from(smth.cos()) * 0.2,
//                 );
//             }
//         }
//     });
//
//     **pose = proposed;
// }
//
// /// Returns true if the position was changed, false if it was not.
// ///
// /// Movement validity rules:
// /// ```text
// ///   From  |   To    | Allowed
// /// --------|---------|--------
// /// in  🧱  | in  🧱  |   ✅
// /// in  🧱  | out 🌫️  |   ✅
// /// out 🌫️  | in  🧱  |   ❌
// /// out 🌫️  | out 🌫️  |   ✅
// /// ```
// /// Only denies movement if starting outside a block and moving into a block.
// /// This prevents players from glitching into blocks while allowing them to move out.
// fn try_change_position(
//     proposed: Vec3,
//     position: &Position,
//     size: EntitySize,
//     blocks: &Blocks,
// ) -> anyhow::Result<()> {
//     // Only check collision if we're starting outside a block
//     if !has_block_collision(position, size, blocks) && has_block_collision(&proposed, size, blocks)
//     {
//         return Err(anyhow::anyhow!("Cannot move into solid blocks"));
//     }
//
//     Ok(())
// }
//
// #[must_use]
// #[allow(clippy::cast_possible_truncation)]
// pub fn is_grounded(position: &Vec3, blocks: &Blocks) -> bool {
//     // Calculate the block position by flooring the x and z coordinates
//     let block_x = position.x as i32;
//     let block_y = (position.y.ceil() - 1.0) as i32; // Check the block directly below
//     let block_z = position.z as i32;
//
//     // Check if the block at the calculated position is not air
//     !blocks
//         .get_block(IVec3::new(block_x, block_y, block_z))
//         .unwrap()
//         .is_air()
// }
//
// fn has_block_collision(position: &Vec3, size: EntitySize, blocks: &Blocks) -> bool {
//     use std::ops::ControlFlow;
//
//     let (min, max) = block_bounds(*position, size);
//     let shrunk = aabb(*position, size).shrink(0.01);
//
//     let res = blocks.get_blocks(min, max, |pos, block| {
//         let pos = Vec3::new(pos.x as f32, pos.y as f32, pos.z as f32);
//
//         for aabb in block.collision_shapes() {
//             let aabb = Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3());
//             let aabb = aabb.move_by(pos);
//
//             if shrunk.collides(&aabb) {
//                 return ControlFlow::Break(false);
//             }
//         }
//
//         ControlFlow::Continue(())
//     });
//
//     res.is_break()
// }
//
// fn look_and_on_ground(
//     &play::LookAndOnGroundC2s { yaw, pitch, .. }: &play::LookAndOnGroundC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     **query.yaw = yaw;
//     **query.pitch = pitch;
//
//     Ok(())
// }
//
// fn position_and_on_ground(
//     &play::PositionAndOnGroundC2s {
//         position,
//         on_ground,
//     }: &play::PositionAndOnGroundC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     change_position_or_correct_client(query, position.as_vec3(), on_ground);
//
//     Ok(())
// }
//
// fn chat_command<'a>(
//     pkt: &play::CommandExecutionC2s<'a>,
//     handle: &dyn LifetimeHandle<'a>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let command = RuntimeLifetime::new(pkt.command.0, handle);
//
//     query.events.push(
//         event::Command {
//             raw: command,
//             by: query.id,
//         },
//         query.world,
//     );
//
//     Ok(())
// }
//
// fn hand_swing(
//     &packet: &play::HandSwingC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     match packet.hand {
//         Hand::Main => {
//             query.animation.push(animation::Kind::SwingMainArm);
//         }
//         Hand::Off => {
//             query.animation.push(animation::Kind::SwingOffHand);
//         }
//     }
//
//     Ok(())
// }
//
// #[instrument(skip_all)]
// fn player_interact_entity(
//     packet: &play::PlayerInteractEntityC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     // attack
//     if packet.interact != EntityInteraction::Attack {
//         return Ok(());
//     }
//
//     let target = packet.entity_id.0;
//     let target = Entity::from_minecraft_id(target);
//
//     query.events.push(
//         event::AttackEntity {
//             origin: query.id,
//             target,
//             damage: 1.0,
//         },
//         query.world,
//     );
//
//     Ok(())
// }
//
// pub struct PacketSwitchQuery<'a> {
//     pub id: Entity,
//     pub handler_registry: &'a HandlerRegistry,
//     pub view: EntityView<'a>,
//     pub compose: &'a Compose,
//     pub io_ref: ConnectionId,
//     pub position: &'a mut Position,
//     pub yaw: &'a mut Yaw,
//     pub pitch: &'a mut Pitch,
//     pub size: &'a mut EntitySize,
//     pub world: &'a World,
//     pub blocks: &'a Blocks,
//     pub pose: &'a mut Pose,
//     pub events: &'a Events,
//     pub confirm_block_sequences: &'a mut ConfirmBlockSequences,
//     pub system: EntityView<'a>,
//     pub inventory: &'a mut hyperion_inventory::PlayerInventory,
//     pub animation: &'a mut ActiveAnimation,
//     pub crafting_registry: &'a hyperion_crafting::CraftingRegistry,
// }
//
// // i.e., shooting a bow, digging a block, etc
// fn player_action(
//     &packet: &play::PlayerActionC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let sequence = packet.sequence.0;
//     let position = IVec3::new(packet.position.x, packet.position.y, packet.position.z);
//
//     match packet.action {
//         PlayerAction::StartDestroyBlock => {
//             let event = event::StartDestroyBlock {
//                 position,
//                 from: query.id,
//                 sequence,
//             };
//             query.events.push(event, query.world);
//         }
//         PlayerAction::StopDestroyBlock => {
//             let event = event::DestroyBlock {
//                 position,
//                 from: query.id,
//                 sequence,
//             };
//
//             query.events.push(event, query.world);
//         }
//         PlayerAction::ReleaseUseItem => {
//             let event = event::ReleaseUseItem {
//                 from: query.id,
//                 item: query.inventory.get_cursor().stack.item,
//             };
//
//             query.id.entity_view(query.world).set(HandStates::new(0));
//
//             query.events.push(event, query.world);
//         }
//         action => bail!("unimplemented {action:?}"),
//     }
//
//     // todo: implement
//
//     Ok(())
// }
//
// // for sneaking/crouching/etc
// fn client_command(
//     &packet: &play::ClientCommandC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     match packet.action {
//         ClientCommand::StartSneaking => {
//             *query.pose = Pose::Sneaking;
//             query.size.height = 1.5;
//         }
//         ClientCommand::StopSneaking | ClientCommand::LeaveBed => {
//             *query.pose = Pose::Standing;
//             query.size.height = 1.8;
//         }
//         ClientCommand::StartSprinting => {
//             query.view.get::<&mut MovementTracking>(|tracking| {
//                 tracking.sprinting = true;
//             });
//         }
//         ClientCommand::StopSprinting => {
//             query.view.get::<&mut MovementTracking>(|tracking| {
//                 tracking.sprinting = false;
//             });
//         }
//         ClientCommand::StartJumpWithHorse
//         | ClientCommand::StopJumpWithHorse
//         | ClientCommand::OpenHorseInventory
//         | ClientCommand::StartFlyingWithElytra => {}
//     }
//
//     Ok(())
// }

/// Handles player interaction with items in hand
///
/// Common uses:
/// - Starting to wind up a bow for shooting arrows
/// - Using consumable items like food or potions
/// - Throwing items like snowballs or ender pearls
/// - Using tools/items with special right-click actions (e.g. fishing rods, shields)
/// - Activating items with duration effects (e.g. chorus fruit teleport)
pub fn player_interact_item(
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

// pub fn player_interact_block(
//     &packet: &play::PlayerInteractBlockC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     // PlayerInteractBlockC2s contains:
//     // - hand: Hand (enum: MainHand or OffHand)
//     // - position: BlockPos (x, y, z coordinates of the block)
//     // - face: Direction (enum: Down, Up, North, South, West, East)
//     // - cursor_position: Vec3 (x, y, z coordinates of cursor on the block face)
//     // - inside_block: bool (whether the player's head is inside a block)
//     // - sequence: VarInt (sequence number for this interaction)
//
//     query.confirm_block_sequences.push(packet.sequence.0);
//
//     let interacted_block_pos = packet.position;
//     let interacted_block_pos_vec = IVec3::new(
//         interacted_block_pos.x,
//         interacted_block_pos.y,
//         interacted_block_pos.z,
//     );
//
//     let Some(interacted_block) = query.blocks.get_block(interacted_block_pos_vec) else {
//         return Ok(());
//     };
//
//     if interacted_block.get(PropName::Open).is_some() {
//         // Toggle the open state of a door
//         // todo: place block instead of toggling door if the player is crouching and holding a
//         // block
//
//         query.events.push(
//             event::ToggleDoor {
//                 position: interacted_block_pos_vec,
//                 from: query.id,
//                 sequence: packet.sequence.0,
//             },
//             query.world,
//         );
//     } else {
//         // Attempt to place a block
//
//         let held = &query.inventory.get_cursor().stack;
//
//         if held.is_empty() {
//             return Ok(());
//         }
//
//         let kind = held.item;
//
//         let Some(block_kind) = BlockKind::from_item_kind(kind) else {
//             warn!("invalid item kind to place: {kind:?}");
//             return Ok(());
//         };
//
//         let block_state = BlockState::from_kind(block_kind);
//
//         let position = interacted_block_pos.get_in_direction(packet.face);
//         let position = IVec3::new(position.x, position.y, position.z);
//
//         let position_dvec3 = position.as_vec3();
//
//         // todo(hack): technically players can do some crazy position stuff to abuse this probably
//         let player_aabb = aabb(**query.position, *query.size);
//
//         let collides_player = block_state
//             .collision_shapes()
//             .map(|aabb| {
//                 Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3()).move_by(position_dvec3)
//             })
//             .any(|block_aabb| Aabb::overlap(&block_aabb, &player_aabb).is_some());
//
//         if collides_player {
//             return Ok(());
//         }
//
//         query.events.push(
//             event::PlaceBlock {
//                 position,
//                 from: query.id,
//                 sequence: packet.sequence.0,
//                 block: block_state,
//             },
//             query.world,
//         );
//     }
//
//     Ok(())
// }
//
// pub fn update_selected_slot(
//     &packet: &play::UpdateSelectedSlotC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     handle_update_selected_slot(packet, query);
//
//     Ok(())
// }
//
// pub fn creative_inventory_action(
//     play::CreativeInventoryActionC2s { slot, clicked_item }: &play::CreativeInventoryActionC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     info!("creative inventory action: {slot} {clicked_item:?}");
//
//     let Ok(slot) = u16::try_from(*slot) else {
//         warn!("invalid slot {slot}");
//         return Ok(());
//     };
//
//     query.inventory.set(slot, clicked_item.clone())?;
//
//     Ok(())
// }
//
// // keywords: inventory
// fn click_slot(
//     pkt: &play::ClickSlotC2s<'_>,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     handle_click_slot(pkt, query);
//
//     Ok(())
// }
//
// fn chat_message<'a>(
//     pkt: &play::ChatMessageC2s<'a>,
//     handle: &dyn LifetimeHandle<'a>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let msg = RuntimeLifetime::new(pkt.message.0, handle);
//
//     query
//         .events
//         .push(event::ChatMessage { msg, by: query.id }, query.world);
//
//     Ok(())
// }
//
// pub fn request_command_completions<'a>(
//     play::RequestCommandCompletionsC2s {
//         transaction_id,
//         text,
//     }: &play::RequestCommandCompletionsC2s<'a>,
//     handle: &dyn LifetimeHandle<'a>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let text = text.0;
//     let transaction_id = transaction_id.0;
//
//     let completion = CommandCompletionRequest {
//         query: text,
//         id: transaction_id,
//     };
//
//     query.handler_registry.trigger(&completion, handle, query)?;
//
//     Ok(())
// }
//
// pub fn client_status(
//     pkt: &play::ClientStatusC2s,
//     handle: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let command = ClientStatusEvent {
//         client: query.id,
//         status: match pkt {
//             play::ClientStatusC2s::PerformRespawn => event::ClientStatusCommand::PerformRespawn,
//             play::ClientStatusC2s::RequestStats => event::ClientStatusCommand::RequestStats,
//         },
//     };
//
//     query.handler_registry.trigger(&command, handle, query)?;
//
//     Ok(())
// }
//
// pub fn confirm_teleportation(
//     pkt: &play::TeleportConfirmC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let entity = query.id.entity_view(query.world);
//
//     entity.get::<Option<&PendingTeleportation>>(|pending_teleport| {
//         if let Some(pending_teleport) = pending_teleport {
//             if VarInt(pending_teleport.teleport_id) != pkt.teleport_id {
//                 return;
//             }
//
//             **query.position = pending_teleport.destination;
//             entity.remove(id::<PendingTeleportation>());
//         }
//     });
//
//     Ok(())
// }
//
// pub fn player_abilities(
//     pkt: &play::UpdatePlayerAbilitiesC2s,
//     _: &dyn LifetimeHandle<'_>,
//     query: &mut PacketSwitchQuery<'_>,
// ) -> anyhow::Result<()> {
//     let entity = query.id.entity_view(query.world);
//
//     entity.get::<&mut Flight>(|flight| match pkt {
//         play::UpdatePlayerAbilitiesC2s::StopFlying => flight.is_flying = false,
//         play::UpdatePlayerAbilitiesC2s::StartFlying => flight.is_flying = flight.allow,
//     });
//     Ok(())
// }

pub fn process_chat(
    mut packets: EventReader<'_, '_, play::ChatMessage>,
    query: Query<'_, '_, &Name>,
) {
    for packet in packets.read() {
        let name = query.get(packet.sender()).unwrap();
        let message = packet.message.as_str();
        info!("{name} sent message {message}");
    }
}
