//! Entity metadata.
//!
//! The base class
//!
//! ```text
//! 0 -> Byte (0)             EntityFlags
//! 1 -> VarInt (1)           AirTicks(300)
//! 2 -> TextComponent? (6)   CustomName("")
//! 3 -> bool (8)             CustomNameVisible(false)
//! 4 -> bool (8)             Silent(false)
//! 5 -> bool (8)             NoGravity(false)
//! 6 -> Pose (21)            Pose(STANDING)
//! 7 -> VarInt (1)           TicksFrozenInPowderSnow(0)
//! ```

use bevy::prelude::*;
use valence_protocol::{Encode, VarInt};
use valence_text::Text;

use crate::{define_and_register_components, simulation::Metadata};

mod flags;
pub use flags::EntityFlags;

// Example usage:
define_and_register_components! {
    1, AirSupply -> VarInt,
    2, CustomName -> Option<Text>,
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

#[derive(Encode, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[derive(Component)]
#[repr(C)] // ideally this would be u8
pub enum Pose {
    #[default]
    Standing,
    FallFlying,
    Sleeping,
    Swimming,
    SpinAttack,
    Sneaking,
    LongJumping,
    Dying,
    Croaking,
    UsingTongue,
    Sitting,
    Roaring,
    Sniffing,
    Emerging,
    Digging,
}

impl Metadata for Pose {
    type Type = Self;

    const INDEX: u8 = 6;

    fn to_type(self) -> Self::Type {
        self
    }
}
