//! | Type | Name | Value | Notes |
//! |------|------|-------|-------|
//! | 0 | Byte | Byte | |
//! | 1 | VarInt | VarInt | |
//! | 2 | VarLong | VarLong | |
//! | 3 | Float | Float | |
//! | 4 | String | String (32767) | |
//! | 5 | Text Component | Text Component | |
//! | 6 | Optional Text Component | (Boolean, Optional Text Component) | Text Component is present if the Boolean is set to true. |
//! | 7 | Slot | Slot | |
//! | 8 | Boolean | Boolean | |
//! | 9 | Rotations | (Float, Float, Float) | rotation on x, rotation on y, rotation on z |
//! | 10 | Position | Position | |
//! | 11 | Optional Position | (Boolean, Optional Position) | Position is present if the Boolean is set to true. |
//! | 12 | Direction | VarInt Enum | Down = 0, Up = 1, North = 2, South = 3, West = 4, East = 5 |
//! | 13 | Optional UUID | (Boolean, Optional UUID) | UUID is present if the Boolean is set to true. |
//! | 14 | Block State | VarInt | An ID in the block state registry. |
//! | 15 | Optional Block State | VarInt | 0 for absent (air is unrepresentable); otherwise, an ID in the block state registry. |
//! | 16 | NBT | NBT | |
//! | 17 | Particle | (VarInt, Varies) | particle type (an ID in the minecraft:particle_type registry), particle data (See Particles.) |
//! | 18 | Particles | (VarInt, Array of (VarInt, Varies)) | length-prefixed list of particle defintions (as above). |
//! | 19 | Villager Data | (VarInt, VarInt, VarInt) | villager type, villager profession, level (See below.) |
//! | 20 | Optional VarInt | VarInt | 0 for absent; 1 + actual value otherwise. Used for entity IDs. |
//! | 21 | Pose | VarInt Enum | STANDING = 0, FALL_FLYING = 1, SLEEPING = 2, SWIMMING = 3, SPIN_ATTACK = 4, SNEAKING = 5, LONG_JUMPING = 6, DYING = 7, CROAKING = 8, USING_TONGUE = 9, SITTING = 10, ROARING = 11, SNIFFING = 12, EMERGING = 13, DIGGING = 14, (1.21.3: SLIDING = 15, SHOOTING = 16, INHALING = 17) |
//! | 22 | Cat Variant | VarInt | An ID in the minecraft:cat_variant registry. |
//! | 23 | Wolf Variant | ID or Wolf Variant | An ID in the minecraft:wolf_variant registry, or an inline definition. |
//! | 24 | Frog Variant | VarInt | An ID in the minecraft:frog_variant registry. |
//! | 25 | Optional Global Position | (Boolean, Optional Identifier, Optional Position) | dimension identifier, position; only if the Boolean is set to true. |
//! | 26 | Painting Variant | ID or Painting Variant | An ID in the minecraft:painting_variant registry, or an inline definition. |
//! | 27 | Sniffer State | VarInt Enum | IDLING = 0, FEELING_HAPPY = 1, SCENTING = 2, SNIFFING = 3, SEARCHING = 4, DIGGING = 5, RISING = 6 |
//! | 28 | Vector3 | (Float, Float, Float) | x, y, z |
//! | 29 | Quaternion | (Float, Float, Float, Float) | x, y, z, w |

use valence_generated::block::BlockState;
use valence_protocol::{ItemStack, VarInt};

use crate::simulation::metadata::entity::Pose;

pub trait MetadataType {
    const INDEX: i32;
}

macro_rules! impl_metadata_type {
    ($($index:expr => $type:ty),* $(,)?) => {
        $(
            impl MetadataType for $type {
                #[allow(clippy::use_self)]
                const INDEX: i32 = $index;
            }
        )*
    };
}

impl_metadata_type! {
    0 => u8,
    1 => VarInt,
    3 => f32,
    7 => ItemStack,
    8 => bool,
    14 => BlockState,
    20 => Pose,
    26 => glam::Vec3,
    27 => glam::Quat,
}
