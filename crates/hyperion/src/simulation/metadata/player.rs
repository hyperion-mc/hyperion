// Index	Type	Meaning	Default
// 15	Float (3)	Additional Hearts	0.0
// 16	VarInt (1)	Score	0
// 17	Byte (0)	The Displayed Skin Parts bit mask that is sent in Client Settings	0
// Bit mask	Meaning
// 0x01	Cape enabled
// 0x02	Jacket enabled
// 0x04	Left sleeve enabled
// 0x08	Right sleeve enabled
// 0x10	Left pants leg enabled
// 0x20	Right pants leg enabled
// 0x40	Hat enabled
// 0x80	Unused
// 18	Byte (0)	Main hand (0 : Left, 1 : Right)	1
// 19	NBT (16)	Left shoulder entity data (for occupying parrot)	Empty
// 20	NBT (16)	Right shoulder entity data (for occupying parrot)	Empty

use flecs_ecs::prelude::*;
use valence_protocol::VarInt;

use super::Metadata;
use crate::define_and_register_components;

// Example usage:
define_and_register_components! {
    // 15	Float (3)	Additional Hearts	0.0
    15, AdditionalHearts -> f32,

    // 16	VarInt (1)	Score	0
    16, Score -> VarInt,

    // 17	Byte (0)	The Displayed Skin Parts bit mask that is sent in Client Settings	0
    17, DisplayedSkinParts -> u8,

    // 18	Byte (0)	Main hand (0 : Left, 1 : Right)	1
    18, MainHand -> u8,

    // 19	NBT (16)	Left shoulder entity data (for occupying parrot)	Empty
    // 19, LeftShoulderEntityData -> Option<nbt::Compound<String>>,

    // 20	NBT (16)	Right shoulder entity data (for occupying parrot)	Empty
    // 20, RightShoulderEntityData -> Option<nbt::Compound<String>>,
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

impl Default for MainHand {
    fn default() -> Self {
        Self::new(1) // 1 = Right hand
    }
}
