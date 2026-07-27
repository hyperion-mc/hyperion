//! Tracked data every entity has (`net.minecraft.world.entity.Entity`).
//!
//! # Where the indices come from
//!
//! A field index is assigned by `SynchedEntityData.defineId`, which counts
//! `defineId` calls per class and offsets each class by the total of its
//! superclasses. Nothing in the protocol carries the numbering, so it cannot be
//! extracted from a packet's stream codec the way `nix/extract-protocol.py`
//! recovers a layout; it was instead read out of the pinned 26.2 server jar by
//! reflecting over each entity class's static `EntityDataAccessor` fields and
//! calling `id()` and `serializer()` on them.
//!
//! That reflection is not yet part of the build, so these tables are
//! hand-written from its output rather than generated. See the module notes on
//! [`crate::simulation::metadata`] for what would make them generated.
//!
//! ```text
//! index  serializer                 accessor
//!     0  Byte (0)                   DATA_SHARED_FLAGS_ID
//!     1  Int (1)                    DATA_AIR_SUPPLY_ID          300
//!     2  OptionalComponent (6)      DATA_CUSTOM_NAME            empty
//!     3  Boolean (8)                DATA_CUSTOM_NAME_VISIBLE    false
//!     4  Boolean (8)                DATA_SILENT                 false
//!     5  Boolean (8)                DATA_NO_GRAVITY             false
//!     6  Pose (20)                  DATA_POSE                   STANDING
//!     7  Int (1)                    DATA_TICKS_FROZEN           0
//! ```
//!
//! The indices are unchanged from 1.20.1. The serializer for `DATA_POSE` is
//! not: `Pose` was 18 and is now 20.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{VarInt, text::Component as TextComponent};

use crate::{define_and_register_components, simulation::Metadata};

mod flags;
pub use flags::EntityFlags;

define_and_register_components! {
    1, AirSupply -> VarInt,
    2, CustomName -> Option<TextComponent<'static>>,
    3, CustomNameVisible -> bool,
    4, Silent -> bool,
    5, NoGravity -> bool,
    7, TicksFrozenInPowderSnow -> VarInt,
}

impl Default for AirSupply {
    fn default() -> Self {
        Self::new(VarInt(300))
    }
}

impl Default for CustomName {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Default for CustomNameVisible {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Default for Silent {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Default for NoGravity {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Default for TicksFrozenInPowderSnow {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

/// `net.minecraft.world.entity.Pose`, sent as its `id` rather than its
/// ordinal; the two agree because the enum numbers itself in declaration
/// order.
///
/// 26.2 added the last three. `Sneaking` is Mojang's `CROUCHING`, kept under
/// the older name because that is what this server's callers spell; the value
/// is 5 either way.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[derive(Component)]
#[flecs(meta)]
#[repr(C)] // ideally this would be u8
pub enum Pose {
    #[default]
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
    Sliding = 15,
    Shooting = 16,
    Inhaling = 17,
}

impl Metadata for Pose {
    type Type = Self;

    const INDEX: u8 = 6;

    fn to_type(self) -> Self::Type {
        self
    }
}

/// `net.minecraft.world.entity.HumanoidArm`, sent as its `id`.
///
/// New as a tracked-data serializer in 26.2: `DATA_PLAYER_MAIN_HAND` was a
/// plain byte through 1.21 and is now this enum, so the value is a `VarInt`
/// rather than one byte even though both spell right-handed as 1.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(C)]
pub enum HumanoidArm {
    Left = 0,
    #[default]
    Right = 1,
}

impl HumanoidArm {
    /// The arm a client reported, defaulting to right-handed for a value no
    /// arm has.
    ///
    /// `ByIdMap.continuous(..., OutOfBoundsStrategy.ZERO)` maps an unknown id
    /// to `LEFT` on the client, so a server that guessed differently would
    /// render a player holding an item in the other hand. Anything but 0 is
    /// right-handed here because the only caller is a client settings packet
    /// whose own default is right.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        if raw == 0 { Self::Left } else { Self::Right }
    }
}
