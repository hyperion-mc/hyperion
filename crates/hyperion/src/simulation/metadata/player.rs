//! Tracked data a player has.
//!
//! Provenance of the indices: see [`super::entity`].
//!
//! ```text
//! index  serializer                 accessor                       class
//!    15  HumanoidArm (42)           DATA_PLAYER_MAIN_HAND          Avatar
//!    16  Byte (0)                   DATA_PLAYER_MODE_CUSTOMISATION Avatar
//!    17  Float (3)                  DATA_PLAYER_ABSORPTION_ID      Player
//!    18  Int (1)                    DATA_SCORE_ID                  Player
//!    19  VillagerData-shaped (19)   DATA_SHOULDER_PARROT_LEFT      Player
//!    20  VillagerData-shaped (19)   DATA_SHOULDER_PARROT_RIGHT     Player
//! ```
//!
//! Every one of these moved in 26.2. `Avatar` is new between `LivingEntity` and
//! `Player`, and it took the main hand and the skin overlay mask with it, so
//! absorption and score each shifted up by two. On 1.20.1 the skin mask was 17
//! and absorption was 15; writing the old numbers puts the skin mask into the
//! absorption field, which renders as a player with no hat and phantom hearts.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::VarInt;

use super::Metadata;
use crate::{define_and_register_components, simulation::metadata::entity::HumanoidArm};

define_and_register_components! {
    // The displayed skin parts bit mask the client sends in its settings:
    // 0x01 cape, 0x02 jacket, 0x04 left sleeve, 0x08 right sleeve,
    // 0x10 left trouser leg, 0x20 right trouser leg, 0x40 hat.
    16, DisplayedSkinParts -> u8,
    17, AdditionalHearts -> f32,
    18, Score -> VarInt,
    // 19 and 20 are the shoulder parrots, which this server never grants.
}

impl Default for AdditionalHearts {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Default for Score {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for DisplayedSkinParts {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Which hand the player holds items in.
///
/// Written out rather than produced by `define_and_register_components!`
/// because the wire type is an enum while callers set it from the raw byte a
/// client settings packet carries.
#[derive(
    Component,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
    derive_more::Deref,
    derive_more::DerefMut
)]
pub struct MainHand {
    value: u8,
}

impl MainHand {
    /// The hand a client's settings packet reported, 0 for left.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self { value }
    }
}

impl Default for MainHand {
    fn default() -> Self {
        Self::new(1) // 1 = Right hand
    }
}

impl Metadata for MainHand {
    type Type = HumanoidArm;

    const INDEX: u8 = 15;

    fn to_type(self) -> Self::Type {
        HumanoidArm::from_raw(self.value)
    }
}
