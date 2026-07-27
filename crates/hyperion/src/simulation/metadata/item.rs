//! Tracked data an item entity has
//! (`net.minecraft.world.entity.item.ItemEntity`).
//!
//! Provenance of the index: see [`super::entity`].
//!
//! ```text
//! index  serializer        accessor     default
//!     8  ItemStack (7)     DATA_ITEM    empty
//! ```
//!
//! The index is unchanged from 1.20.1; the serializer moved from 6 to 7, and
//! the stack itself gained a component patch in 1.20.5.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::item::Slot;

use super::Metadata;
use crate::define_and_register_components;

define_and_register_components! {
    8, Item -> Slot<'static>
}

impl Default for Item {
    fn default() -> Self {
        Self::new(Slot::Empty)
    }
}
