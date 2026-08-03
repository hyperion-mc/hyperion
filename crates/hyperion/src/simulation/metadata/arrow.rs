//! Tracked data every arrow has
//! (`net.minecraft.world.entity.projectile.arrow.AbstractArrow`).
//!
//! Provenance of the indices: see [`super::entity`]. Read out of the pinned
//! 26.2 jar the same way, by reflecting over `AbstractArrow`'s static
//! `EntityDataAccessor` fields and calling `id()` on each after
//! `SharedConstants.tryDetectVersion()` and `Bootstrap.bootStrap()`. The same
//! run reproduced `super::living_entity`'s table (8..14) unchanged, which is
//! what says the method is reading the same numbering that one was built from.
//!
//! It agrees with counting the declarations: `Entity` declares eight accessors
//! (`Entity.java:284-298`), `Projectile` declares none, and `AbstractArrow`'s
//! three follow at `AbstractArrow.java:71-73` in that order.
//!
//! ```text
//! index  serializer                 accessor
//!     8  Byte (0)                   ID_FLAGS                    0
//!     9  Byte (0)                   PIERCE_LEVEL                0
//!    10  Boolean (8)                IN_GROUND                   false
//! ```
//!
//! Only index 10 is here. The other two exist on the wire and nothing this
//! server does writes them: 8 is the crit and no-physics flag pair
//! (`AbstractArrow.java:74-75`) and 9 is piercing, which needs an enchantment.

use flecs_ecs::prelude::*;

use super::Metadata;
use crate::define_and_register_components;

define_and_register_components! {
    // 8 ID_FLAGS and 9 PIERCE_LEVEL are deliberately absent; see the module
    // note. Leaving a gap rather than defining a field nothing writes keeps
    // the table to what this server actually sends.
    10, InGround -> bool,
}

impl Default for InGround {
    fn default() -> Self {
        Self::new(false)
    }
}
