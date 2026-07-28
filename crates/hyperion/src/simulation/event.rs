//! Flecs components which are used for events.

use flecs_ecs::{core::Entity, macros::Component};
use glam::{IVec3, Vec3};
use hyperion_utils::Lifetime;
use valence_bytes::Utf8Bytes;
use valence_generated::block::BlockState;
use valence_protocol::{
    Hand, ItemStack,
    packets::play::click_slot_c2s::{ClickMode, SlotChange},
};
use valence_server::ItemKind;

use super::blocks::RayCollision;
use crate::{net::ProxyId, simulation::skin::PlayerSkin};

#[derive(Component, Default, Debug)]
pub struct ItemDropEvent {
    pub item: ItemStack,
    pub location: Vec3,
}

#[derive(Component, Default, Debug)]
pub struct ItemInteract {
    pub entity: Entity,
    pub hand: Hand,
    pub sequence: i32,
}

pub struct ChatMessage {
    pub msg: Utf8Bytes,
    pub by: Entity,
}

#[derive(Debug)]
pub struct SetSkin {
    pub skin: PlayerSkin,
    pub by: Entity,
}

/// Represents an attack action by an entity in the game.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AttackEntity {
    /// The entity that is performing the attack.
    pub origin: Entity,
    pub target: Entity,
    /// The damage dealt by the attack. This corresponds to the same unit as [`crate::simulation::metadata::living_entity::Health`].
    pub damage: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StartDestroyBlock {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DestroyBlock {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PlaceBlock {
    pub position: IVec3,
    pub block: BlockState,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ToggleDoor {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Copy, Clone, Debug)]
pub struct SwingArm {
    pub hand: Hand,
}

#[derive(Copy, Clone, Debug)]
pub struct ReleaseUseItem {
    pub from: Entity,
    pub item: ItemKind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
#[expect(missing_docs, reason = "self explanatory")]
pub enum Posture {
    Standing = 0,
    FallFlying = 1,
    Sleeping = 2,
    Swimming = 3,
    SpinAttack = 4,
    Sneaking = 5,
    LongJumping = 6,
    Dying = 7,
    Croaking = 8,
    UsingTongue = 9,
    Sitting = 10,
    Roaring = 11,
    Sniffing = 12,
    Emerging = 13,
    Digging = 14,
}

/// <https://wiki.vg/index.php?title=Protocol&oldid=18375#Set_Entity_Metadata>
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PostureUpdate {
    /// The new posture of the entity.
    pub state: Posture,
}

pub struct Command {
    pub raw: Utf8Bytes,
    pub by: Entity,
}

/// A player right-clicked an entity.
///
/// `minecraft:interact`, which since 26.2 means nothing else: attacking left
/// with the packet split out into `minecraft:attack`. A game whose furniture is
/// alive -- a shopkeeper, a kit selector's mobs -- has no other way to hear a
/// click on it, and until this existed the packet reached the dispatch table
/// and fell straight through it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EntityInteract {
    /// The entity that was clicked.
    pub target: Entity,
    pub from: Entity,
    pub hand: Hand,
}

/// A player right-clicked a block.
///
/// Raised for every `use_item_on`, before the door and block-placement
/// branches decide whether they also want it. Those two are the only other
/// things that packet can mean, and both of them are about changing the world;
/// a block that is a button, a shop or a kit selector is a block whose click
/// has to be heard even though nothing about the world changes, and until this
/// carried a position there was no way to hear one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BlockInteract {
    /// The block that was clicked, not the space in front of it.
    pub position: IVec3,
    pub from: Entity,
    pub hand: Hand,
    pub sequence: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientStatusCommand {
    PerformRespawn,
    RequestStats,
}

#[derive(Clone, Debug)]
pub struct ClientStatusEvent {
    pub client: Entity,
    pub status: ClientStatusCommand,
}

unsafe impl Lifetime for ClientStatusEvent {
    type WithLifetime<'a> = Self;
}

#[derive(Clone, Debug)]
pub struct ProjectileEntityEvent {
    pub client: Entity,
    pub projectile: Entity,
}

#[derive(Clone, Debug)]
pub struct ProjectileBlockEvent {
    pub collision: RayCollision,
    pub projectile: Entity,
}

#[derive(Clone, Debug)]
pub struct ClickSlotEvent {
    pub client: Entity,
    pub window_id: u8,
    pub state_id: i32,
    pub slot: i16,
    pub button: i8,
    pub mode: ClickMode,
    pub slot_changes: Vec<SlotChange>,
    pub carried_item: ItemStack,
}

#[derive(Clone, Debug)]
pub struct DropItemStackEvent {
    pub client: Entity,
    pub from_slot: Option<i16>,
    pub item: ItemStack,
}

#[derive(Clone, Debug)]
pub struct UpdateSelectedSlotEvent {
    pub client: Entity,
    pub slot: u8,
}

#[derive(Clone, Debug)]
pub struct HitGroundEvent {
    pub client: Entity,
    /// This is at least 3
    pub fall_distance: f32,
}

/// A proxy has asked for the packets needed to make a player subscribed to this channel see the
/// channel's entity.
///
/// The answer belongs to the proxy that asked and to no other: it subscribes that proxy's players,
/// and a proxy that never asked has no player waiting for it. So the asker is carried here rather
/// than inferred later, and [`crate::net::intermediate`] filters the reply on it.
#[derive(Copy, Clone, Debug)]
pub struct RequestSubscribeChannelPackets {
    /// The channel entity whose spawn packets were asked for.
    pub channel: Entity,
    /// The proxy that asked.
    pub receiver: ProxyId,
}
