use flecs_ecs::prelude::*;
use valence_generated::block::BlockState;

use super::Metadata;
use crate::define_and_register_components;

// Example usage:
define_and_register_components! {
    22, DisplayedBlockState -> BlockState,
}

impl Default for DisplayedBlockState {
    fn default() -> Self {
        Self::new(BlockState::EMERALD_BLOCK)
    }
}
