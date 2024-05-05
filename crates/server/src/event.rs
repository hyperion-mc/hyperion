use std::{alloc::Allocator, borrow::Cow, cell::RefCell, fmt::Debug};

use bumpalo::Bump;
use derive_more::{Deref, DerefMut};
use evenio::{component::Component, entity::EntityId, event::Event};
use glam::Vec3;
use rayon_local::RayonLocal;
use valence_generated::{block::BlockState, status_effects::StatusEffect};
use valence_protocol::{packets::play::click_slot_c2s::SlotChange, BlockPos, Hand};
use valence_server::{entity::EntityKind, ItemStack};
use valence_text::Text;

use crate::{
    components::FullEntityPose,
    net::{Server, MAX_PACKET_SIZE},
    util::player_skin::PlayerSkin,
};

#[derive(Event, Debug)]
/// An event that is sent when a player clicks in the inventory.
pub struct ClickEvent {
    #[event(target)]
    pub by: EntityId,
    pub click_type: ClickType,
    pub carried_item: ItemStack,
}

/// The type of click that the player performed.
#[derive(Clone, Debug)]
pub enum ClickType {
    LeftClick {
        slot: i16,
        // todo: left click only can result in 1 slot change right?
        slot_change: SlotChange,
    },
    RightClick {
        slot: i16,
        // todo: left click only can result in 1 slot change right?
        slot_change: SlotChange,
    },
    LeftClickOutsideOfWindow,
    RightClickOutsideOfWindow,
    ShiftLeftClick {
        slot: i16,
        // todo: should be 2 slot changes right?
        slot_changes: [SlotChange; 2],
    },
    ShiftRightClick {
        slot: i16,
        // todo: should be 2 slot changes right?
        slot_changes: [SlotChange; 2],
    },
    HotbarKeyPress {
        button: i8,
        slot: i16,
        // todo: should be 2 slot changes right?
        slot_changes: [SlotChange; 2],
    },
    OffHandSwap {
        slot: i16,
        // todo: should be 2 slot changes right?
        slot_changes: [SlotChange; 2],
    },
    // todo: support for creative mode
    CreativeMiddleClick {
        slot: i16,
    },
    QDrop {
        slot: i16,
        // todo: left click only can result in 1 slot change right?
        slot_change: SlotChange,
    },
    QControlDrop {
        slot: i16,
        // todo: left click only can result in 1 slot change right?
        slot_change: SlotChange,
    },
    StartLeftMouseDrag,
    StartRightMouseDrag,
    StartMiddleMouseDrag,
    AddSlotLeftDrag {
        slot: i16,
    },
    AddSlotRightDrag {
        slot: i16,
    },
    AddSlotMiddleDrag {
        slot: i16,
    },
    EndLeftMouseDrag {
        slot_changes: Vec<SlotChange>,
    },
    EndRightMouseDrag {
        slot_changes: Vec<SlotChange>,
    },
    EndMiddleMouseDrag,
    DoubleClick {
        slot: i16,
        slot_changes: Vec<SlotChange>,
    },
    DoubleClickReverseOrder {
        slot: i16,
        slot_changes: Vec<SlotChange>,
    },
}

#[derive(Event)]
/// An event that is sent when a player is changes his main hand
pub struct UpdateSelectedSlot {
    #[event(target)]
    pub id: EntityId,
    pub slot: usize,
}

/// This event is sent when the payer equipment gets sent to the client.
#[derive(Event)]
pub struct UpdateEquipment {
    #[event(target)]
    pub id: EntityId,
}

/// Initialize a Minecraft entity (like a zombie) with a given pose.
#[derive(Event)]
pub struct InitEntity {
    /// The pose of the entity.
    pub pose: FullEntityPose,
    pub display: EntityKind,
}

#[derive(Event)]
pub struct Command {
    #[event(target)]
    pub by: EntityId,
    pub raw: String,
}

#[derive(Event)]
pub struct PlayerInit {
    #[event(target)]
    pub target: EntityId,

    /// The name of the player i.e., `Emerald_Explorer`.
    pub username: Box<str>,
    pub pose: FullEntityPose,
}

/// Sent whenever a player joins the server.
#[derive(Event)]
pub struct PlayerJoinWorld {
    /// The [`EntityId`] of the player.
    #[event(target)]
    pub target: EntityId,
}

/// An event that is sent whenever a player is kicked from the server.
#[derive(Event)]
pub struct KickPlayer {
    /// The [`EntityId`] of the player.
    #[event(target)] // Works on tuple struct fields as well.
    pub target: EntityId,
    /// The reason the player was kicked.
    pub reason: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Pose {
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

#[derive(Event)]
#[event(immutable)]
pub struct PoseUpdate {
    #[event(target)]
    pub target: EntityId,
    pub state: Pose,
}

/// An event that is sent whenever a player swings an arm.
#[derive(Event)]
pub struct SwingArm {
    /// The [`EntityId`] of the player.
    #[event(target)]
    pub target: EntityId,
    /// The hand the player is swinging.
    pub hand: Hand,
}

#[derive(Event)]
pub struct HurtEntity {
    #[event(target)]
    pub target: EntityId,
    pub damage: f32,
}

pub enum AttackType {
    Shove,
    Melee,
}

#[derive(Event)]
pub struct AttackEntity {
    /// The [`EntityId`] of the player.
    #[event(target)]
    pub target: EntityId,
    /// The location of the player that is hitting.
    pub from_pos: Vec3,
    pub from: EntityId,
    pub damage: f32,
    pub source: AttackType,
}

#[derive(Event)]
#[event(immutable)]
pub struct Death {
    #[event(target)]
    pub target: EntityId,
}

/// An event to kill all minecraft entities (like zombies, skeletons, etc). This will be sent to the equivalent of
/// `/killall` in the game.
#[derive(Event)]
pub struct KillAllEntities;

#[derive(Event)]
pub struct Teleport {
    #[event(target)]
    pub target: EntityId,
    pub position: Vec3,
}

/// i.e., when zombies bump into another player
#[derive(Event)]
pub struct Shoved {
    #[event(target)]
    pub target: EntityId,
    pub from: EntityId,
    pub from_location: Vec3,
}

/// An event when server stats are updated.
#[derive(Event)]
pub struct Stats {
    /// The number of milliseconds per tick in the last second.
    pub ms_per_tick_mean_1s: f64,
    /// The number of milliseconds per tick in the last 5 seconds.
    pub ms_per_tick_mean_5s: f64,
}

// todo: naming? this seems bad
#[derive(Debug)]
pub struct Scratch<A: Allocator = std::alloc::Global> {
    inner: Vec<u8, A>,
}

impl Scratch {
    #[must_use]
    pub fn new() -> Self {
        let inner = Vec::with_capacity(MAX_PACKET_SIZE);
        Self { inner }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Nice for getting a buffer that can be used for intermediate work
///
/// # Safety
/// - every single time [`ScratchBuffer::obtain`] is called, the buffer will be cleared before returning
/// - the buffer has capacity of at least `MAX_PACKET_SIZE`
pub unsafe trait ScratchBuffer: sealed::Sealed + Debug {
    type Allocator: Allocator;
    fn obtain(&mut self) -> &mut Vec<u8, Self::Allocator>;
}

mod sealed {
    pub trait Sealed {}
}

impl<A: Allocator + Debug> sealed::Sealed for Scratch<A> {}

unsafe impl<A: Allocator + Debug> ScratchBuffer for Scratch<A> {
    type Allocator = A;

    fn obtain(&mut self) -> &mut Vec<u8, Self::Allocator> {
        self.inner.clear();
        &mut self.inner
    }
}

pub type BumpScratch<'a> = Scratch<&'a Bump>;

impl<A: Allocator> From<A> for Scratch<A> {
    fn from(allocator: A) -> Self {
        Self {
            inner: Vec::with_capacity_in(MAX_PACKET_SIZE, allocator),
        }
    }
}

#[derive(Event)]
pub struct BlockStartBreak {
    #[event(target)]
    pub by: EntityId,
    pub position: BlockPos,
    pub sequence: i32,
}

#[derive(Event)]
pub struct BlockAbortBreak {
    #[event(target)]
    pub by: EntityId,
    pub position: BlockPos,
    pub sequence: i32,
}

#[derive(Event)]
pub struct BlockFinishBreak {
    #[event(target)]
    pub by: EntityId,
    pub position: BlockPos,
    pub sequence: i32,
}

#[derive(Event, Debug)]
pub struct UpdateBlock {
    pub position: BlockPos,
    pub id: BlockState,
    pub sequence: i32,
}

#[derive(Event)]
pub struct ChatMessage {
    #[event(target)]
    pub target: EntityId,
    pub message: Text,
}

#[derive(Event)]
pub struct DisguisePlayer {
    #[event(target)]
    pub target: EntityId,
    pub mob: EntityKind,
}

#[derive(Component, Deref, DerefMut, Default)]
pub struct Scratches {
    inner: RayonLocal<RefCell<Scratch>>,
}

/// This often only displays the effect. For instance, for speed it does not give the actual speed effect.
#[derive(Event, Copy, Clone)]
pub struct DisplayPotionEffect {
    #[event(target)]
    pub target: EntityId,
    pub effect: StatusEffect,
    pub amplifier: u8,
    pub duration: i32,

    // todo: make this a bitfield
    ///  whether or not this is an effect provided by a beacon and therefore should be less intrusive on the screen.
    /// Optional, and defaults to false.
    pub ambient: bool,
    pub show_particles: bool,
    pub show_icon: bool,
}

#[derive(Event, Copy, Clone)]
pub struct SpeedEffect {
    #[event(target)]
    target: EntityId,
    level: u8,
}

impl SpeedEffect {
    #[must_use]
    pub const fn new(target: EntityId, level: u8) -> Self {
        Self { target, level }
    }

    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }
}

// todo: why need two life times?
#[derive(Event)]
pub struct Gametick;

/// An event that is sent when it is time to send packets to clients.
#[derive(Event)]
pub struct Egress<'a> {
    pub server: &'a mut Server,
}

#[derive(Event)]
pub struct SetPlayerSkin {
    #[event(target)]
    pub target: EntityId,
    pub skin: PlayerSkin,
}
