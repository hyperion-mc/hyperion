//! Tracked data a block display has
//! (`net.minecraft.world.entity.Display$BlockDisplay`).
//!
//! Provenance of the index: see [`super::entity`].
//!
//! ```text
//! index  serializer        accessor              default
//!    23  BlockState (14)   DATA_BLOCK_STATE_ID   air
//! ```
//!
//! 23 rather than the 22 this used to send: `Display` gained
//! `DATA_POS_ROT_INTERPOLATION_DURATION`, so everything below it shifted.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::block_state;

use super::Metadata;
use crate::{define_and_register_components, simulation::metadata::r#type::BlockStateId};

define_and_register_components! {
    23, DisplayedBlockState -> BlockStateId,
}

impl Default for DisplayedBlockState {
    fn default() -> Self {
        Self::new(BlockStateId(
            block_state::default_state_id("minecraft:emerald_block")
                .expect("minecraft:emerald_block is in every version's block table"),
        ))
    }
}
