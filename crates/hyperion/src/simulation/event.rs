use bevy::prelude::*;
use glam::{IVec3, Vec3};
use valence_generated::block::BlockState;
use valence_protocol::{
    Hand, Ident, ItemStack,
    packets::play::{
        ParticleS2c,
        click_slot_c2s::{ClickMode, SlotChange},
    },
};

use super::blocks::RayCollision;
use crate::simulation::skin::PlayerSkin;

// TODO: Check that all of these events are needed

#[derive(Event, Default, Debug)]
pub struct ItemDropEvent {
    pub item: ItemStack,
    pub location: Vec3,
}

#[derive(Event, Debug)]
pub struct ItemInteract {
    pub entity: Entity,
    pub hand: Hand,
    pub sequence: i32,
}

#[derive(Event, Debug)]
pub struct SetSkin {
    pub skin: PlayerSkin,
    pub by: Entity,
}

/// Represents an attack action by a player in the game. This attack may not be succesful such as
/// when a player attempts to attack a teammate.
#[derive(Event, Clone, Debug)]
pub struct AttackEntity {
    /// The player that is performing the attack. This can be indirect, such as the player who
    /// fired an arrow.
    pub origin: Entity,
    /// The entity that is being attacked.
    pub target: Entity,
    /// The direction of the attack. This value is normalized. This is used to calculate knockback.
    pub direction: Vec3,
    /// The damage dealt by the attack. This corresponds to the same unit as [`crate::simulation::metadata::living_entity::Health`].
    pub damage: f32,
    /// Sound to play on a successful attack
    pub sound: Ident,
    /// Particles to broadcast to all clients except the origin. The origin may already have
    /// generated these particles locally
    pub particles: Option<ParticleS2c<'static>>,
}

#[derive(Event, Copy, Clone, Debug, PartialEq, Eq)]
pub struct StartDestroyBlock {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Event, Copy, Clone, Debug, PartialEq, Eq)]
pub struct DestroyBlock {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Event, Copy, Clone, Debug, PartialEq, Eq)]
pub struct PlaceBlock {
    pub position: IVec3,
    pub block: BlockState,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Event, Copy, Clone, Debug, PartialEq, Eq)]
pub struct ToggleDoor {
    pub position: IVec3,
    pub from: Entity,
    pub sequence: i32,
}

#[derive(Event, Copy, Clone, Debug)]
pub struct SwingArm {
    pub hand: Hand,
}

#[derive(Event, Copy, Clone, Debug)]
pub struct ReleaseUseItem {
    pub from: Entity,
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
#[derive(Event, Copy, Clone, Debug, PartialEq, Eq)]
pub struct PostureUpdate {
    /// The new posture of the entity.
    pub state: Posture,
}

#[derive(Event)]
pub struct BlockInteract {}

#[derive(Event, Clone, Debug)]
pub struct ProjectileEntityEvent {
    pub client: Entity,
    pub projectile: Entity,
}

#[derive(Event, Clone, Debug)]
pub struct ProjectileBlockEvent {
    pub collision: RayCollision,
    pub projectile: Entity,
}

#[derive(Event, Clone, Debug)]
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

#[derive(Event, Clone, Debug)]
pub struct DropItemStackEvent {
    pub client: Entity,
    pub from_slot: Option<i16>,
    pub item: ItemStack,
}

#[derive(Event, Clone, Debug)]
pub struct UpdateSelectedSlotEvent {
    pub client: Entity,
    pub slot: u8,
}

#[derive(Event, Clone, Debug)]
pub struct HitGroundEvent {
    pub client: Entity,
    /// This is at least 3
    pub fall_distance: f32,
}

#[derive(Event, Clone, Debug)]
pub struct InteractEvent {
    pub client: Entity,
    pub hand: Hand,
    pub sequence: i32,
}
