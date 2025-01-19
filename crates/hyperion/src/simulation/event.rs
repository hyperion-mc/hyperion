//! Flecs components which are used for events.

use derive_more::Constructor;
use flecs_ecs::{core::Entity, macros::Component};
use glam::{IVec3, Vec3};
use hyperion_utils::{Lifetime, RuntimeLifetime};
use valence_generated::block::BlockState;
use valence_protocol::Hand;
use valence_server::{ItemKind, entity::item_frame::ItemStack};

use super::blocks::RayCollision;
use crate::simulation::skin::PlayerSkin;

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
    pub msg: RuntimeLifetime<&'static str>,
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

#[derive(Copy, Clone, Debug, PartialEq, Constructor)]
pub struct HealthUpdate {
    pub from: f32,
    pub to: f32,
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
    pub raw: RuntimeLifetime<&'static str>,
    pub by: Entity,
}

pub struct BlockInteract {}

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
