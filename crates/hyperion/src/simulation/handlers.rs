//! <https://wiki.vg/index.php?title=Protocol&oldid=18375>

// The RegistryHandler requires a specific function signature
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::trivially_copy_pass_by_ref)]

use std::borrow::Cow;

use anyhow::{Context, bail};
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, World};
use geometry::aabb::Aabb;
use glam::{IVec3, Vec3};
use hyperion_utils::{EntityExt, LifetimeHandle, RuntimeLifetime};
use tracing::{info, instrument, warn};
use valence_generated::{
    block::{BlockKind, BlockState, PropName},
    item::ItemKind,
};
use valence_protocol::{
    Hand, ItemStack, VarInt,
    packets::play::{
        self, client_command_c2s::ClientCommand, player_action_c2s::PlayerAction,
        player_interact_entity_c2s::EntityInteraction,
        player_position_look_s2c::PlayerPositionLookFlags,
    },
};
use valence_text::IntoText;

use super::{
    ConfirmBlockSequences, EntitySize, Position,
    animation::{self, ActiveAnimation},
    block_bounds,
    blocks::Blocks,
    bow::BowCharging,
    event::ClientStatusEvent,
};
use crate::{
    net::{Compose, ConnectionId, decoder::BorrowedPacketFrame},
    simulation::{
        Pitch, Yaw, aabb,
        event::{self, PluginMessage},
        metadata::entity::Pose,
        packet::HandlerRegistry,
    },
    storage::{CommandCompletionRequest, Events, InteractEvent},
};

fn full(
    &play::FullC2s {
        position,
        yaw,
        pitch,
        ..
    }: &play::FullC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    // check to see if the player is moving too fast
    // if they are, ignore the packet

    let position = position.as_vec3();
    change_position_or_correct_client(query, position);

    query.yaw.yaw = yaw;
    query.pitch.pitch = pitch;

    Ok(())
}

// #[instrument(skip_all)]
fn change_position_or_correct_client(query: &mut PacketSwitchQuery<'_>, proposed: Vec3) {
    let pose = &mut *query.position;

    if let Err(e) = try_change_position(proposed, pose, *query.size, query.blocks) {
        // Send error message to player
        let msg = format!("§c{e}");
        let pkt = play::GameMessageS2c {
            chat: msg.into_cow_text(),
            overlay: false,
        };

        if let Err(e) = query.compose.unicast(&pkt, query.io_ref, query.system) {
            warn!("Failed to send error message to player: {e}");
        }

        // Correct client position
        let pkt = play::PlayerPositionLookS2c {
            position: pose.position.as_dvec3(),
            yaw: query.yaw.yaw,
            pitch: query.pitch.pitch,
            flags: PlayerPositionLookFlags::default(),
            teleport_id: VarInt(fastrand::i32(..)),
        };

        if let Err(e) = query.compose.unicast(&pkt, query.io_ref, query.system) {
            warn!("Failed to correct client position: {e}");
        }
    }
}

/// Returns true if the position was changed, false if it was not.
/// The vanilla server has a max speed of 100 blocks per tick.
/// However, we are much more conservative.
const MAX_BLOCKS_PER_TICK: f32 = 30.0;

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
    position: &mut Position,
    size: EntitySize,
    blocks: &Blocks,
) -> anyhow::Result<()> {
    is_within_speed_limits(**position, proposed)?;

    // Only check collision if we're starting outside a block
    if !has_block_collision(position, size, blocks) && has_block_collision(&proposed, size, blocks)
    {
        return Err(anyhow::anyhow!("Cannot move into solid blocks"));
    }

    **position = proposed;
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
        .unwrap()
        .is_air()
}
fn is_within_speed_limits(current: Vec3, proposed: Vec3) -> anyhow::Result<()> {
    let delta = proposed - current;
    if delta.length_squared() > MAX_BLOCKS_PER_TICK.powi(2) {
        return Err(anyhow::anyhow!(
            "Moving too fast! Maximum speed is {MAX_BLOCKS_PER_TICK} blocks per tick"
        ));
    }
    Ok(())
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

fn look_and_on_ground(
    &play::LookAndOnGroundC2s { yaw, pitch, .. }: &play::LookAndOnGroundC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    **query.yaw = yaw;
    **query.pitch = pitch;

    Ok(())
}

fn position_and_on_ground(
    &play::PositionAndOnGroundC2s { position, .. }: &play::PositionAndOnGroundC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    change_position_or_correct_client(query, position.as_vec3());

    Ok(())
}

fn chat_command<'a>(
    pkt: &play::CommandExecutionC2s<'a>,
    handle: &dyn LifetimeHandle<'a>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let command = RuntimeLifetime::new(pkt.command.0, handle);

    query.events.push(
        event::Command {
            raw: command,
            by: query.id,
        },
        query.world,
    );

    Ok(())
}

fn hand_swing(
    &packet: &play::HandSwingC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    match packet.hand {
        Hand::Main => {
            query.animation.push(animation::Kind::SwingMainArm);
        }
        Hand::Off => {
            query.animation.push(animation::Kind::SwingOffHand);
        }
    }

    Ok(())
}

#[instrument(skip_all)]
fn player_interact_entity(
    packet: &play::PlayerInteractEntityC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    // attack
    if packet.interact != EntityInteraction::Attack {
        return Ok(());
    }

    let target = packet.entity_id.0;
    let target = Entity::from_minecraft_id(target);

    query.events.push(
        event::AttackEntity {
            origin: query.id,
            target,
            damage: 1.0,
        },
        query.world,
    );

    Ok(())
}

pub struct PacketSwitchQuery<'a> {
    pub id: Entity,
    pub handler_registry: &'a HandlerRegistry,
    pub view: EntityView<'a>,
    pub compose: &'a Compose,
    pub io_ref: ConnectionId,
    pub position: &'a mut Position,
    pub yaw: &'a mut Yaw,
    pub pitch: &'a mut Pitch,
    pub size: &'a mut EntitySize,
    pub world: &'a World,
    pub blocks: &'a Blocks,
    pub pose: &'a mut Pose,
    pub events: &'a Events,
    pub confirm_block_sequences: &'a mut ConfirmBlockSequences,
    pub system: EntityView<'a>,
    pub inventory: &'a mut hyperion_inventory::PlayerInventory,
    pub animation: &'a mut ActiveAnimation,
    pub crafting_registry: &'a hyperion_crafting::CraftingRegistry,
}

// i.e., shooting a bow, digging a block, etc
fn player_action(
    &packet: &play::PlayerActionC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let sequence = packet.sequence.0;
    let position = IVec3::new(packet.position.x, packet.position.y, packet.position.z);

    match packet.action {
        PlayerAction::StartDestroyBlock => {
            let event = event::StartDestroyBlock {
                position,
                from: query.id,
                sequence,
            };
            query.events.push(event, query.world);
        }
        PlayerAction::StopDestroyBlock => {
            let event = event::DestroyBlock {
                position,
                from: query.id,
                sequence,
            };

            query.events.push(event, query.world);
        }
        PlayerAction::ReleaseUseItem => {
            let event = event::ReleaseUseItem {
                from: query.id,
                item: query.inventory.get_cursor().item,
            };

            query.events.push(event, query.world);
        }
        action => bail!("unimplemented {action:?}"),
    }

    // todo: implement

    Ok(())
}

// for sneaking/crouching/etc
fn client_command(
    &packet: &play::ClientCommandC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    match packet.action {
        ClientCommand::StartSneaking => {
            *query.pose = Pose::Sneaking;
            query.size.height = 1.5;
        }
        ClientCommand::StopSneaking | ClientCommand::LeaveBed => {
            *query.pose = Pose::Standing;
            query.size.height = 1.8;
        }
        ClientCommand::StartSprinting
        | ClientCommand::StopSprinting
        | ClientCommand::StartJumpWithHorse
        | ClientCommand::StopJumpWithHorse
        | ClientCommand::OpenHorseInventory
        | ClientCommand::StartFlyingWithElytra => {}
    }

    Ok(())
}

/// Handles player interaction with items in hand
///
/// Common uses:
/// - Starting to wind up a bow for shooting arrows
/// - Using consumable items like food or potions
/// - Throwing items like snowballs or ender pearls
/// - Using tools/items with special right-click actions (e.g. fishing rods, shields)
/// - Activating items with duration effects (e.g. chorus fruit teleport)
pub fn player_interact_item(
    &play::PlayerInteractItemC2s { hand, sequence }: &play::PlayerInteractItemC2s,
    handle: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let event = InteractEvent {
        hand,
        sequence: sequence.0,
    };

    let cursor = query.inventory.get_cursor();

    if !cursor.is_empty() {
        if cursor.item == ItemKind::WrittenBook {
            let packet = play::OpenWrittenBookS2c { hand };
            query.compose.unicast(&packet, query.io_ref, query.system)?;
        } else if cursor.item == ItemKind::Bow {
            // Start charging bow
            let entity = query.world.entity_from_id(query.id);
            entity.get::<Option<&BowCharging>>(|charging| {
                if charging.is_some() {
                    return;
                }
                entity.set(BowCharging::now());
            });
        }
    }

    query.handler_registry.trigger(&event, handle, query)?;

    Ok(())
}

pub fn player_interact_block(
    &packet: &play::PlayerInteractBlockC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    // PlayerInteractBlockC2s contains:
    // - hand: Hand (enum: MainHand or OffHand)
    // - position: BlockPos (x, y, z coordinates of the block)
    // - face: Direction (enum: Down, Up, North, South, West, East)
    // - cursor_position: Vec3 (x, y, z coordinates of cursor on the block face)
    // - inside_block: bool (whether the player's head is inside a block)
    // - sequence: VarInt (sequence number for this interaction)

    query.confirm_block_sequences.push(packet.sequence.0);

    let interacted_block_pos = packet.position;
    let interacted_block_pos_vec = IVec3::new(
        interacted_block_pos.x,
        interacted_block_pos.y,
        interacted_block_pos.z,
    );

    let Some(interacted_block) = query.blocks.get_block(interacted_block_pos_vec) else {
        return Ok(());
    };

    if interacted_block.get(PropName::Open).is_some() {
        // Toggle the open state of a door
        // todo: place block instead of toggling door if the player is crouching and holding a
        // block

        query.events.push(
            event::ToggleDoor {
                position: interacted_block_pos_vec,
                from: query.id,
                sequence: packet.sequence.0,
            },
            query.world,
        );
    } else {
        // Attempt to place a block

        let held = query.inventory.get_cursor();

        if held.is_empty() {
            return Ok(());
        }

        let kind = held.item;

        let Some(block_kind) = BlockKind::from_item_kind(kind) else {
            warn!("invalid item kind to place: {kind:?}");
            return Ok(());
        };

        let block_state = BlockState::from_kind(block_kind);

        let position = interacted_block_pos.get_in_direction(packet.face);
        let position = IVec3::new(position.x, position.y, position.z);

        let position_dvec3 = position.as_vec3();

        // todo(hack): technically players can do some crazy position stuff to abuse this probably
        let player_aabb = aabb(**query.position, *query.size);

        let collides_player = block_state
            .collision_shapes()
            .map(|aabb| {
                Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3()).move_by(position_dvec3)
            })
            .any(|block_aabb| Aabb::overlap(&block_aabb, &player_aabb).is_some());

        if collides_player {
            return Ok(());
        }

        query.events.push(
            event::PlaceBlock {
                position,
                from: query.id,
                sequence: packet.sequence.0,
                block: block_state,
            },
            query.world,
        );
    }

    Ok(())
}

pub fn update_selected_slot(
    &play::UpdateSelectedSlotC2s { slot }: &play::UpdateSelectedSlotC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    query.inventory.set_cursor(slot);

    Ok(())
}

pub fn creative_inventory_action(
    play::CreativeInventoryActionC2s { slot, clicked_item }: &play::CreativeInventoryActionC2s,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    info!("creative inventory action: {slot} {clicked_item:?}");

    let Ok(slot) = u16::try_from(*slot) else {
        warn!("invalid slot {slot}");
        return Ok(());
    };

    query.inventory.set(slot, clicked_item.clone())?;

    Ok(())
}

pub fn custom_payload<'a>(
    packet: &play::CustomPayloadC2s<'a>,
    handle: &dyn LifetimeHandle<'a>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let channel = packet.channel.clone().into_inner();

    let Cow::Borrowed(borrow) = channel else {
        bail!("NO")
    };

    let event = PluginMessage {
        channel: RuntimeLifetime::new(borrow, handle),
        data: RuntimeLifetime::new(packet.data.0.0, handle),
    };

    query.events.push(event, query.world);

    Ok(())
}

// keywords: inventory
fn click_slot(
    pkt: &play::ClickSlotC2s<'_>,
    _: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let to_send_pkt = play::ScreenHandlerSlotUpdateS2c {
        window_id: -1,
        state_id: VarInt::default(),
        slot_idx: 0, // crafting result
        slot_data: Cow::Borrowed(&ItemStack::EMPTY),
    };

    // negate click
    query
        .compose
        .unicast(&to_send_pkt, query.io_ref, query.system)?;

    let slot_idx = u16::try_from(pkt.slot_idx).context("slot index is negative")?;

    let item_in_slot = query.inventory.get(slot_idx)?;

    let to_send_pkt = play::ScreenHandlerSlotUpdateS2c {
        window_id: 0,
        state_id: VarInt::default(),
        slot_idx: pkt.slot_idx,
        slot_data: Cow::Borrowed(item_in_slot),
    };

    query
        .compose
        .unicast(&to_send_pkt, query.io_ref, query.system)?;

    // info!("click slot\n{pkt:#?}");

    // // todo(security): validate the player can do this. This is a MAJOR security issue.
    // // as players will be able to spawn items in their inventory wit current logic.
    // for SlotChange { idx, stack } in pkt.slot_changes.iter() {
    //     let idx = u16::try_from(*idx).context("slot index is negative")?;
    //     query.inventory.set(idx, stack.clone())?;
    // }

    let item = query.inventory.crafting_result(query.crafting_registry);

    let set_item_pkt = play::ScreenHandlerSlotUpdateS2c {
        window_id: 0,
        state_id: VarInt(0),
        slot_idx: 0, // crafting result
        slot_data: Cow::Owned(item),
    };

    query
        .compose
        .unicast(&set_item_pkt, query.io_ref, query.system)?;

    Ok(())
}

fn chat_message<'a>(
    pkt: &play::ChatMessageC2s<'a>,
    handle: &dyn LifetimeHandle<'a>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let msg = RuntimeLifetime::new(pkt.message.0, handle);

    query
        .events
        .push(event::ChatMessage { msg, by: query.id }, query.world);

    Ok(())
}

pub fn request_command_completions<'a>(
    play::RequestCommandCompletionsC2s {
        transaction_id,
        text,
    }: &play::RequestCommandCompletionsC2s<'a>,
    handle: &dyn LifetimeHandle<'a>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let text = text.0;
    let transaction_id = transaction_id.0;

    let completion = CommandCompletionRequest {
        query: text,
        id: transaction_id,
    };

    query.handler_registry.trigger(&completion, handle, query)?;

    Ok(())
}

pub fn client_status(
    pkt: &play::ClientStatusC2s,
    handle: &dyn LifetimeHandle<'_>,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let command = ClientStatusEvent {
        client: query.id,
        status: match pkt {
            play::ClientStatusC2s::PerformRespawn => event::ClientStatusCommand::PerformRespawn,
            play::ClientStatusC2s::RequestStats => event::ClientStatusCommand::RequestStats,
        },
    };

    query.handler_registry.trigger(&command, handle, query)?;

    Ok(())
}

pub fn add_builtin_handlers(registry: &mut HandlerRegistry) {
    registry.add_handler(Box::new(chat_message));
    registry.add_handler(Box::new(click_slot));
    registry.add_handler(Box::new(client_command));
    registry.add_handler(Box::new(client_status));
    registry.add_handler(Box::new(chat_command));
    registry.add_handler(Box::new(creative_inventory_action));
    registry.add_handler(Box::new(custom_payload));
    registry.add_handler(Box::new(full));
    registry.add_handler(Box::new(hand_swing));
    registry.add_handler(Box::new(look_and_on_ground));
    registry.add_handler(Box::new(player_action));
    registry.add_handler(Box::new(player_interact_block));
    registry.add_handler(Box::new(player_interact_entity));
    registry.add_handler(Box::new(player_interact_item));
    registry.add_handler(Box::new(position_and_on_ground));
    registry.add_handler(Box::new(request_command_completions));
    registry.add_handler(Box::new(update_selected_slot));
}

/// # Safety
/// The [`BorrowedPacketFrame`] must borrow data from the [`Compose::bump`] in
/// [`PacketSwitchQuery::Compose`]
pub unsafe fn packet_switch<'a>(
    raw: BorrowedPacketFrame<'a>,
    query: &mut PacketSwitchQuery<'a>,
) -> anyhow::Result<()> {
    let packet_id = raw.id;
    let data = raw.body;

    // SAFETY: The only data that HandlerRegistry::process_packet is aware of outliving 'a is the packet bytes.
    // The packet bytes are stored in the compose bump allocator.
    // LifetimeTracker::assert_no_references will be called on the bump tracker before the
    // bump allocator is cleared.
    let handle = unsafe { query.compose.bump_tracker.handle() };
    let handle: &dyn LifetimeHandle<'a> = &handle;

    query
        .handler_registry
        .process_packet(packet_id, data, handle, query)?;

    Ok(())
}
