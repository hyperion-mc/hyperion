//! Tracked data every living entity has
//! (`net.minecraft.world.entity.LivingEntity`).
//!
//! Provenance of the indices: see [`super::entity`].
//!
//! ```text
//! index  serializer                 accessor
//!     8  Byte (0)                   DATA_LIVING_ENTITY_FLAGS    0
//!     9  Float (3)                  DATA_HEALTH_ID              1.0
//!    10  Particles (17)             DATA_EFFECT_PARTICLES       empty
//!    11  Boolean (8)                DATA_EFFECT_AMBIENCE_ID     false
//!    12  Int (1)                    DATA_ARROW_COUNT_ID         0
//!    13  Int (1)                    DATA_STINGER_COUNT_ID       0
//!    14  OptionalBlockPos (11)      SLEEPING_POS_ID             empty
//! ```
//!
//! Index 10 is the one that changed shape rather than position: through 1.20.1
//! it was a packed potion colour sent as an `Int`, and since 1.20.5 it is the
//! list of particles to render. A colour written there is read as a particle
//! count.

use std::fmt::Display;

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::VarInt;

use super::Metadata;
use crate::define_and_register_components;

define_and_register_components! {
    // Hand states, used to trigger the blocking, eating and drinking
    // animations: 0x01 hand active, 0x02 offhand rather than main, 0x04
    // riptide spin attack.
    8, HandStates -> u8,
    9, Health -> f32,
    // 10 DATA_EFFECT_PARTICLES is the list of particles a potion effect
    // renders. Nothing here brews one, so it stays at its empty default; it is
    // called out because through 1.20.1 that index was a packed potion colour
    // sent as an `Int`, and a colour written there now reads as a particle
    // count.
    11, IsPotionEffectAmbient -> bool,
    12, ArrowsInEntity -> VarInt,
    13, BeeStingersInEntity -> VarInt,
    // 14 SLEEPING_POS_ID is an optional block position; nothing here sleeps.
}

impl Default for HandStates {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Default for IsPotionEffectAmbient {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Default for BeeStingersInEntity {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for ArrowsInEntity {
    fn default() -> Self {
        Self::new(VarInt(0))
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::new(20.0)
    }
}

impl Health {
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.value <= 0.0
    }

    pub fn damage(&mut self, damage: f32) {
        self.update(self.value - damage);
    }

    const fn update(&mut self, value: f32) {
        self.value = value.clamp(0.0, 20.0);
    }

    pub fn heal(&mut self, heal: f32) {
        self.update(self.value + heal);
    }
}

// use unicode hearts
impl Display for Health {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "we want saturating ceiling"
        )]
        let normal = usize::try_from(self.value.ceil() as isize).unwrap_or(0);

        let full_hearts = normal / 2;
        for _ in 0..full_hearts {
            write!(f, "\u{E001}")?;
        }

        if normal % 2 == 1 {
            // half heart
            write!(f, "\u{E002}")?;
        }

        Ok(())
    }
}
