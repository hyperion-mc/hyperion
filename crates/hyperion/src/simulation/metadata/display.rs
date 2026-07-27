//! Tracked data a display entity has (`net.minecraft.world.entity.Display`).
//!
//! Provenance of the indices: see [`super::entity`].
//!
//! ```text
//! index  serializer      accessor                                              default
//!     8  Int (1)         DATA_TRANSFORMATION_INTERPOLATION_START_DELTA_TICKS   0
//!     9  Int (1)         DATA_TRANSFORMATION_INTERPOLATION_DURATION            0
//!    10  Int (1)         DATA_POS_ROT_INTERPOLATION_DURATION                   0
//!    11  Vector3 (39)    DATA_TRANSLATION                                      (0, 0, 0)
//!    12  Vector3 (39)    DATA_SCALE                                            (1, 1, 1)
//!    13  Quaternion (40) DATA_LEFT_ROTATION                                    identity
//!    14  Quaternion (40) DATA_RIGHT_ROTATION                                   identity
//!    15  Byte (0)        DATA_BILLBOARD_RENDER_CONSTRAINTS                     0
//!    16  Int (1)         DATA_BRIGHTNESS_OVERRIDE                              -1
//!    17  Float (3)       DATA_VIEW_RANGE                                       1.0
//!    18  Float (3)       DATA_SHADOW_RADIUS                                    0.0
//!    19  Float (3)       DATA_SHADOW_STRENGTH                                  1.0
//!    20  Float (3)       DATA_WIDTH                                            0.0
//!    21  Float (3)       DATA_HEIGHT                                           0.0
//!    22  Int (1)         DATA_GLOW_COLOR_OVERRIDE                              -1
//! ```
//!
//! The port also fixes a numbering that was already wrong for 1.20.1: index 10
//! was omitted here, so every field from the translation onwards was sent one
//! index low and a block display's scale arrived as its translation.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::VarInt;

use super::Metadata;
use crate::define_and_register_components;

define_and_register_components! {
    8, InterpolationDelay -> VarInt,
    9, InterpolationDuration -> VarInt,
    10, PosRotInterpolationDuration -> VarInt,
    11, Translation -> glam::Vec3,
    12, Scale -> glam::Vec3,
    13, RotationLeft -> glam::Quat,
    14, RotationRight -> glam::Quat,
    15, BillboardConstraints -> u8,
    16, BrightnessOverride -> VarInt,
    17, ViewRange -> f32,
    18, ShadowRadius -> f32,
    19, ShadowStrength -> f32,
    20, Width -> f32,
    21, Height -> f32,
    22, GlowColorOverride -> VarInt,
}

impl Default for InterpolationDelay {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for InterpolationDuration {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for PosRotInterpolationDuration {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for Translation {
    fn default() -> Self {
        Self::new(glam::Vec3::ZERO)
    }
}

impl Default for Scale {
    fn default() -> Self {
        Self::new(glam::Vec3::ONE)
    }
}

impl Default for RotationLeft {
    fn default() -> Self {
        Self::new(glam::Quat::IDENTITY)
    }
}

impl Default for RotationRight {
    fn default() -> Self {
        Self::new(glam::Quat::IDENTITY)
    }
}

impl Default for BillboardConstraints {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Default for BrightnessOverride {
    fn default() -> Self {
        Self::new(VarInt(-1))
    }
}

impl Default for ViewRange {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Default for ShadowRadius {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Default for ShadowStrength {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Default for Width {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Default for Height {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Default for GlowColorOverride {
    fn default() -> Self {
        Self::new(VarInt(-1))
    }
}
