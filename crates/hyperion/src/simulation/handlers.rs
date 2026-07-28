//! Decoding and acting on the packets a playing client sends.
//!
//! Every id here comes from
//! [`hyperion_minecraft_proto::generated::packet_id::play::serverbound::PacketId`],
//! which is generated from Minecraft 26.2's own data (protocol 776). The ids
//! are not the ones valence uses for 1.20.1: `chat` moved from 5 to 9,
//! `swing` from 47 to 63, `container_click` from 11 to 18, and the movement
//! packets from 20..=23 to 30..=33. A stale table does not error, it runs the
//! wrong handler on bytes that parse, which is why [`route`] resolves an id
//! through the generated enum rather than through integer literals, and why
//! anything it cannot place is logged rather than dropped.

// Every handler shares one signature so that `route` can hand back a function
// pointer, which leaves several of them returning a Result they never fail.
#![allow(clippy::unnecessary_wraps)]

use anyhow::bail;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, World, id};
use geometry::aabb::Aabb;
use glam::{DVec3, IVec3, Vec3};
use hyperion_minecraft_proto::{
    generated::packet_id::play::{
        clientbound::PacketId as ClientboundPacketId, serverbound::PacketId,
    },
    packets::play::{
        clientbound::OpenBook,
        entity::Interact,
        serverbound::{self as c2s, client_command, player_action, player_command},
    },
    types::{Direction, HumanoidArm, InteractionHand},
};
use hyperion_utils::EntityExt;
use tracing::warn;
use valence_generated::{
    block::{BlockKind, BlockState, PropName},
    item::ItemKind,
};
use valence_protocol::{Hand, packets::play};

use super::{
    ConfirmBlockSequences, EntitySize, Flight, MovementTracking, PendingTeleportation, Position,
    animation::{self, ActiveAnimation},
    block_bounds,
    blocks::Blocks,
    event::ClientStatusEvent,
    inventory::{handle_close_window, handle_update_selected_slot},
};
use crate::{
    net::{
        Compose, ConnectionId, PROTOCOL_VERSION, agnostic,
        decoder::BorrowedPacketFrame,
        protocol::{decode_body, frame_body, send},
    },
    simulation::{
        Pitch, Yaw, aabb, event, gamemode,
        metadata::{
            entity::Pose,
            living_entity::HandStates,
            player::{DisplayedSkinParts, MainHand},
        },
        packet::{HandlerRegistry, serverbound},
    },
    storage::{CommandCompletionRequest, Events, InteractEvent},
};

/// `Player.STANDING_DIMENSIONS` height.
///
/// The server holds the hitbox heights itself because the client only reports
/// which keys are down, never how tall it thinks it is.
const STANDING_HEIGHT: f32 = 1.8;

/// Height of the `Pose.CROUCHING` entry in `Player.POSES`.
const CROUCHING_HEIGHT: f32 = 1.5;

/// Bit 0 of the movement packets' trailing flags byte.
///
/// 26.2 replaced `boolean onGround` with a byte whose second bit reports a
/// horizontal collision, so reading the field as a bool would treat a player
/// who is airborne but scraping a wall as standing on the ground.
const ON_GROUND: i8 = 1;

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
    pub inventory: &'a mut hyperion_inventory::PlayerInventory,
    pub animation: &'a mut ActiveAnimation,
    pub crafting_registry: &'a hyperion_crafting::CraftingRegistry,
}

/// Reads one packet body and applies it.
///
/// Every handler starts by decoding the body, so the type it decodes into sits
/// next to the behaviour that depends on it rather than in a table somewhere
/// else.
type Handler = fn(&[u8], &mut PacketSwitchQuery<'_>) -> anyhow::Result<()>;

/// What this server does with a serverbound play id.
pub enum Route {
    /// Decoded and acted on.
    Act(Handler),
    /// Well-formed and deliberately dropped; the arm in [`route`] says why.
    Ignore,
    /// A 776 packet nothing here reads yet.
    Unhandled(PacketId),
    /// An id protocol 776 does not define at all.
    Unknown(i32),
}

/// Decide what to do with a serverbound play frame, from its id alone.
///
/// Split out from [`packet_switch`] so the id table can be checked without a
/// world to dispatch into. The failure this defends against is a table from
/// the wrong protocol version: it produces no error, it runs a handler that
/// was written for a different packet over bytes that happen to parse.
#[must_use]
pub fn route(id: i32) -> Route {
    let Some(packet) = PacketId::from_raw(id) else {
        return Route::Unknown(id);
    };

    match packet {
        PacketId::AcceptTeleportation => Route::Act(accept_teleportation),
        PacketId::Attack => Route::Act(attack),
        PacketId::Chat => Route::Act(chat),
        PacketId::ChatCommand => Route::Act(chat_command),
        PacketId::ClientCommand => Route::Act(client_command),
        PacketId::ClientInformation => Route::Act(client_information),
        PacketId::CommandSuggestion => Route::Act(command_suggestion),
        PacketId::Interact => Route::Act(interact),
        PacketId::ContainerClose => Route::Act(container_close),
        PacketId::MovePlayerPos => Route::Act(move_player_pos),
        PacketId::MovePlayerPosRot => Route::Act(move_player_pos_rot),
        PacketId::MovePlayerRot => Route::Act(move_player_rot),
        PacketId::MovePlayerStatusOnly => Route::Act(move_player_status_only),
        PacketId::PlayerAbilities => Route::Act(player_abilities),
        PacketId::PlayerAction => Route::Act(player_action),
        PacketId::PlayerCommand => Route::Act(player_command),
        PacketId::PlayerInput => Route::Act(player_input),
        PacketId::SetCarriedItem => Route::Act(set_carried_item),
        PacketId::Swing => Route::Act(swing),
        PacketId::UseItem => Route::Act(use_item),
        PacketId::UseItemOn => Route::Act(use_item_on),

        // Read and dropped on purpose. Each of these is something a healthy
        // client sends on its own schedule and that this server has no state
        // to update for, so warning about them would bury the ids that matter:
        //
        // - `client_tick_end` arrives every tick, `chunk_batch_received` after
        //   every batch, and this server sends chunks without pacing them.
        // - `keep_alive`, `pong` and `ping_request` are liveness only; nothing
        //   here times a connection out on them yet.
        // - `chat_ack` and `chat_session_update` belong to signed chat, which
        //   this server does not verify.
        // - `player_loaded` is the client saying its terrain finished loading,
        //   which the join sequence does not wait on.
        // - `custom_payload`, `cookie_response` and `resource_pack` answer
        //   things this server never asks for.
        // - the recipe book and advancement settings are client-side UI state.
        PacketId::ChatAck
        | PacketId::ChatSessionUpdate
        | PacketId::ChunkBatchReceived
        | PacketId::ClientTickEnd
        | PacketId::CookieResponse
        | PacketId::CustomPayload
        | PacketId::KeepAlive
        | PacketId::PingRequest
        | PacketId::PlayerLoaded
        | PacketId::Pong
        | PacketId::RecipeBookChangeSettings
        | PacketId::RecipeBookSeenRecipe
        | PacketId::ResourcePack
        | PacketId::SeenAdvancements => Route::Ignore,

        // Everything else, including any variant a later protocol adds to this
        // non-exhaustive enum. Reaching this arm is a gap, not a no-op, so
        // packet_switch says so.
        other => Route::Unhandled(other),
    }
}

/// Decodes `frame` and applies it to the player who sent it.
///
/// # Errors
/// Returns an error when the body does not match the layout its id promises.
pub fn packet_switch(
    frame: BorrowedPacketFrame,
    query: &mut PacketSwitchQuery<'_>,
) -> anyhow::Result<()> {
    let body = frame_body(&frame);

    match route(frame.id) {
        Route::Act(handler) => handler(body, query),
        Route::Ignore => Ok(()),
        Route::Unhandled(packet) => {
            warn!(
                id = frame.id,
                bytes = body.len(),
                "no handler for serverbound play packet {packet:?}"
            );
            Ok(())
        }
        Route::Unknown(id) => {
            warn!(
                id,
                bytes = body.len(),
                "serverbound play id is not in the protocol {PROTOCOL_VERSION} table"
            );
            Ok(())
        }
    }
}

fn accept_teleportation(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::AcceptTeleportation(teleport_id) = decode_body(body)?;
    let entity = query.id.entity_view(query.world);

    entity.get::<Option<&PendingTeleportation>>(|pending| {
        // A stale id means the client is confirming a teleport this server has
        // already replaced; moving it would undo the newer one.
        if let Some(pending) = pending
            && pending.teleport_id == teleport_id
        {
            **query.position = pending.destination;
            entity.remove(id::<PendingTeleportation>());
        }
    });

    Ok(())
}

/// 26.2 split attacking out of `interact` into its own packet, so this is the
/// whole of the melee path: `interact` now only ever means a right-click.
fn attack(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::Attack = decode_body(body)?;

    query.events.push(
        event::AttackEntity {
            origin: query.id,
            target: Entity::from_minecraft_id(packet.entity_id),
            damage: 1.0,
        },
        query.world,
    );

    Ok(())
}

/// A right-click on an entity.
///
/// The mirror of [`attack`], which 26.2 split out of this packet. Nothing here
/// decides what the click means: the position on the entity and the sneak flag
/// are dropped because no caller has wanted them, and the game is left to
/// resolve the target however it likes.
fn interact(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: Interact = decode_body(body)?;

    query.events.push(
        event::EntityInteract {
            target: Entity::from_minecraft_id(packet.entity_id),
            from: query.id,
            hand: hand(packet.hand),
        },
        query.world,
    );

    Ok(())
}

fn chat(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: serverbound::Chat<'_> = decode_body(body)?;

    query.events.push(
        event::ChatMessage {
            msg: packet.message.to_owned().into(),
            by: query.id,
        },
        query.world,
    );

    Ok(())
}

fn chat_command(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::ChatCommand(command) = decode_body(body)?;

    query.events.push(
        event::Command {
            raw: command.to_owned().into(),
            by: query.id,
        },
        query.world,
    );

    Ok(())
}

fn client_command(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::ClientCommand(action) = decode_body(body)?;

    let status = match action {
        client_command::Action::PerformRespawn => event::ClientStatusCommand::PerformRespawn,
        client_command::Action::RequestStats => event::ClientStatusCommand::RequestStats,
        // Added in 26.2 for the client's gamerule screen. This server keeps no
        // per-player gamerule overrides, so there is nothing to answer with.
        client_command::Action::RequestGameruleValues => return Ok(()),
    };

    query.handler_registry.trigger(
        &ClientStatusEvent {
            client: query.id,
            status,
        },
        query,
    )
}

/// The client tells the server which of its own skin layers to render, and the
/// server has to echo that back as entity metadata or nobody sees them --
/// including the player themselves in third person. Without this the metadata
/// keeps its default of 0, so every player appears with the base layer only:
/// no hat, no jacket, no sleeves.
///
/// Sent again in play whenever the player changes a video setting;
/// [`crate::net::protocol::pre_play`] handles the copy sent in configuration.
fn client_information(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let info: c2s::ClientInformation<'_> = decode_body(body)?;

    // `ClientInformation` reads the mask with `readUnsignedByte`; the generated
    // body types it as `i8`, and the metadata field wants the same eight bits
    // back out.
    let displayed = info.model_customisation.cast_unsigned();

    let main_hand = match info.main_hand {
        HumanoidArm::Left => 0,
        HumanoidArm::Right => 1,
    };

    query
        .view
        .set(DisplayedSkinParts::new(displayed))
        .set(MainHand::new(main_hand));

    Ok(())
}

fn command_suggestion(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::CommandSuggestion<'_> = decode_body(body)?;

    query.handler_registry.trigger(
        &CommandCompletionRequest {
            query: packet.command.to_owned().into(),
            id: packet.id,
        },
        query,
    )
}

fn container_close(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::ContainerClose(_container_id) = decode_body(body)?;

    handle_close_window(query);

    Ok(())
}

fn move_player_pos(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::MovePlayerPos = decode_body(body)?;

    change_position_or_correct_client(
        query,
        DVec3::new(packet.x, packet.y, packet.z).as_vec3(),
        packet.on_ground & ON_GROUND != 0,
    );

    Ok(())
}

fn move_player_pos_rot(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::MovePlayerPosRot = decode_body(body)?;

    change_position_or_correct_client(
        query,
        DVec3::new(packet.x, packet.y, packet.z).as_vec3(),
        packet.on_ground & ON_GROUND != 0,
    );

    **query.yaw = packet.y_rot;
    **query.pitch = packet.x_rot;

    Ok(())
}

fn move_player_rot(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::MovePlayerRot = decode_body(body)?;

    **query.yaw = packet.y_rot;
    **query.pitch = packet.x_rot;

    Ok(())
}

/// The client sends this when only the ground flag changed, which is how a
/// standing player reports landing. Feeding it through the same path as a move
/// keeps the fall and jump bookkeeping in
/// [`change_position_or_correct_client`] from missing that transition.
fn move_player_status_only(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::MovePlayerStatusOnly(flags) = decode_body(body)?;

    let unchanged = **query.position;
    change_position_or_correct_client(query, unchanged, flags & ON_GROUND != 0);

    Ok(())
}

fn player_abilities(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: serverbound::PlayerAbilities = decode_body(body)?;

    query.view.get::<&mut Flight>(|flight| {
        flight.is_flying = packet.is_flying() && flight.allow;
    });

    Ok(())
}

// i.e., shooting a bow, digging a block, etc
/// Tell the client the world did not change, and which of its guesses to drop.
///
/// A client predicts a break or a place locally and holds the prediction until
/// the server answers for that sequence number. Say nothing and the block stays
/// missing on their screen until something else resends the chunk, which looks
/// exactly like a server that lost the packet.
///
/// The ack goes out here rather than through [`ConfirmBlockSequences`], which
/// nothing drains, or through `Blocks::to_confirm`, which this query holds only
/// a shared reference to. A refusal is decided and final at this point, so
/// there is nothing for a queue to add.
fn refuse_block_change(sequence: i32, query: &PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    send(
        query.compose,
        query.io_ref,
        ClientboundPacketId::BlockChangedAck.to_raw(),
        &hyperion_minecraft_proto::packets::play::clientbound::BlockChangedAck(sequence),
    )
}

fn player_action(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::PlayerAction = decode_body(body)?;

    let position = IVec3::new(packet.pos.x, packet.pos.y, packet.pos.z);

    // Adventure and spectator do not build. A vanilla client in either mode
    // never sends the dig at all, so reaching here means either a mode change
    // the client has not applied yet or a client that is not vanilla; both are
    // refused the same way.
    if !gamemode::of(query.view).may_build()
        && matches!(
            packet.action,
            player_action::Action::StartDestroyBlock
                | player_action::Action::AbortDestroyBlock
                | player_action::Action::StopDestroyBlock
        )
    {
        return refuse_block_change(packet.sequence, query);
    }

    match packet.action {
        player_action::Action::StartDestroyBlock => {
            query.events.push(
                event::StartDestroyBlock {
                    position,
                    from: query.id,
                    sequence: packet.sequence,
                },
                query.world,
            );
        }
        player_action::Action::StopDestroyBlock => {
            query.events.push(
                event::DestroyBlock {
                    position,
                    from: query.id,
                    sequence: packet.sequence,
                },
                query.world,
            );
        }
        player_action::Action::ReleaseUseItem => {
            let event = event::ReleaseUseItem {
                from: query.id,
                item: query.inventory.held().stack.item,
            };

            query.id.entity_view(query.world).set(HandStates::new(0));

            query.events.push(event, query.world);
        }
        action => bail!("unimplemented {action:?}"),
    }

    Ok(())
}

fn player_command(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::PlayerCommand = decode_body(body)?;

    match packet.action {
        player_command::Action::StartSprinting => {
            query.view.get::<&mut MovementTracking>(|tracking| {
                tracking.sprinting = true;
            });
        }
        player_command::Action::StopSprinting => {
            query.view.get::<&mut MovementTracking>(|tracking| {
                tracking.sprinting = false;
            });
        }
        player_command::Action::StopSleeping => {
            *query.pose = Pose::Standing;
            query.size.height = STANDING_HEIGHT;
        }
        player_command::Action::StartRidingJump
        | player_command::Action::StopRidingJump
        | player_command::Action::OpenInventory
        | player_command::Action::StartFallFlying => {}
    }

    Ok(())
}

/// Which movement keys are down, including sneak.
///
/// 26.2 dropped `PRESS_SHIFT_KEY`/`RELEASE_SHIFT_KEY` from
/// `ServerboundPlayerCommandPacket`, so this is the only packet that reports
/// crouching. A server that reads only player commands leaves every player
/// standing, at full hitbox height, however much they crouch.
fn player_input(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let input: serverbound::PlayerInput = decode_body(body)?;

    if input.shift() {
        *query.pose = Pose::Sneaking;
        query.size.height = CROUCHING_HEIGHT;
    } else if *query.pose == Pose::Sneaking {
        *query.pose = Pose::Standing;
        query.size.height = STANDING_HEIGHT;
    }

    Ok(())
}

fn set_carried_item(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::SetCarriedItem(slot) = decode_body(body)?;

    // `simulation::inventory` is still written against valence's 763 packets.
    // The hotbar index is a plain integer in both, so the one-field body is
    // rebuilt here rather than duplicating the slot bookkeeping; the rest of
    // that module needs the 776 port before `container_click` can work.
    handle_update_selected_slot(
        play::UpdateSelectedSlotC2s {
            slot: u16::try_from(slot)?,
        },
        query,
    );

    Ok(())
}

fn swing(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let c2s::Swing(hand) = decode_body(body)?;

    query.animation.push(match hand {
        InteractionHand::MainHand => animation::Kind::SwingMainArm,
        InteractionHand::OffHand => animation::Kind::SwingOffHand,
    });

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
fn use_item(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::UseItem = decode_body(body)?;
    let hand = hand(packet.hand);

    let held = &query.inventory.held().stack;

    if !held.is_empty() {
        if held.item == ItemKind::WrittenBook {
            send(
                query.compose,
                query.io_ref,
                ClientboundPacketId::OpenBook.to_raw(),
                &OpenBook(packet.hand),
            )?;
        }

        query.events.push(
            event::ItemInteract {
                entity: query.id,
                hand,
                sequence: packet.sequence,
            },
            query.world,
        );
    }

    query.handler_registry.trigger(
        &InteractEvent {
            hand,
            sequence: packet.sequence,
        },
        query,
    )
}

fn use_item_on(body: &[u8], query: &mut PacketSwitchQuery<'_>) -> anyhow::Result<()> {
    let packet: c2s::UseItemOn = decode_body(body)?;

    query.confirm_block_sequences.push(packet.sequence);

    let hit = packet.block_hit;
    let interacted = IVec3::new(hit.block_pos.x, hit.block_pos.y, hit.block_pos.z);

    // Raised for the click itself, ahead of the two branches below that ask
    // what the click should *change*. A game whose blocks are interactive --
    // a kit selector, a shop, a button -- needs the position of the block that
    // was clicked and nothing else, and neither `ToggleDoor` nor `PlaceBlock`
    // will fire for it: the first wants an openable block and the second wants
    // a placeable item in hand, so an empty-handed click on a quartz plinth
    // used to reach the world as nothing at all.
    query.events.push(
        event::BlockInteract {
            position: interacted,
            from: query.id,
            hand: hand(packet.hand),
            sequence: packet.sequence,
        },
        query.world,
    );

    let Some(interacted_block) = query.blocks.get_block(interacted) else {
        return Ok(());
    };

    if interacted_block.get(PropName::Open).is_some() {
        // Toggle the open state of a door
        // todo: place block instead of toggling door if the player is crouching and holding a
        // block

        query.events.push(
            event::ToggleDoor {
                position: interacted,
                from: query.id,
                sequence: packet.sequence,
            },
            query.world,
        );

        return Ok(());
    }

    // Adventure and spectator do not place, for the same reason they do not
    // dig. The door above is deliberately on the other side of this check:
    // `mayBuild` gates building, and a vanilla adventure player can still open
    // a door.
    if !gamemode::of(query.view).may_build() {
        return refuse_block_change(packet.sequence, query);
    }

    // Attempt to place a block
    let held = &query.inventory.held().stack;

    if held.is_empty() {
        return Ok(());
    }

    let kind = held.item;

    let Some(block_kind) = BlockKind::from_item_kind(kind) else {
        warn!("invalid item kind to place: {kind:?}");
        return Ok(());
    };

    let block_state = BlockState::from_kind(block_kind);
    let position = interacted + offset(hit.direction);

    // todo(hack): technically players can do some crazy position stuff to abuse this probably
    let player_aabb = aabb(**query.position, *query.size);

    let collides_player = block_state
        .collision_shapes()
        .map(|aabb| {
            Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3()).move_by(position.as_vec3())
        })
        .any(|block_aabb| Aabb::overlap(&block_aabb, &player_aabb).is_some());

    if collides_player {
        return Ok(());
    }

    query.events.push(
        event::PlaceBlock {
            position,
            from: query.id,
            sequence: packet.sequence,
            block: block_state,
        },
        query.world,
    );

    Ok(())
}

/// The unit step away from a block face, for placing against it.
const fn offset(direction: Direction) -> IVec3 {
    match direction {
        Direction::Down => IVec3::new(0, -1, 0),
        Direction::Up => IVec3::new(0, 1, 0),
        Direction::North => IVec3::new(0, 0, -1),
        Direction::South => IVec3::new(0, 0, 1),
        Direction::West => IVec3::new(-1, 0, 0),
        Direction::East => IVec3::new(1, 0, 0),
    }
}

/// The proto crate's hand and valence's are the same two values in the same
/// order; the simulation events still speak valence's.
const fn hand(hand: InteractionHand) -> Hand {
    match hand {
        InteractionHand::MainHand => Hand::Main,
        InteractionHand::OffHand => Hand::Off,
    }
}

// #[instrument(skip_all)]
fn change_position_or_correct_client(
    query: &mut PacketSwitchQuery<'_>,
    proposed: Vec3,
    on_ground: bool,
) {
    let pose = &mut *query.position;

    if let Err(e) = try_change_position(proposed, pose, *query.size, query.blocks) {
        // Send error message to player
        let pkt = agnostic::chat(format!("§c{e}"));

        if let Err(e) = query.compose.unicast(&pkt, query.io_ref) {
            warn!("Failed to send error message to player: {e}");
        }

        query
            .id
            .entity_view(query.world)
            .set(PendingTeleportation::new(**pose));
    }
    query.view.get::<&mut MovementTracking>(|tracking| {
        tracking.received_movement_packets = tracking.received_movement_packets.saturating_add(1);
        let y_delta = proposed.y - pose.y;

        if y_delta > 0. && tracking.was_on_ground && !on_ground {
            tracking.server_velocity.y = 0.419_999_986_886_978_15;

            if tracking.sprinting {
                let smth = **query.yaw * 0.017_453_292;
                tracking.server_velocity += DVec3::new(
                    f64::from(-smth.sin()) * 0.2,
                    0.0,
                    f64::from(smth.cos()) * 0.2,
                );
            }
        }
    });

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
    let is_air = blocks
        .get_block(IVec3::new(block_x, block_y, block_z))
        .is_none_or(BlockState::is_air);

    !is_air
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
