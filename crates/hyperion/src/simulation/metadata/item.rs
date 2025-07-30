use bevy::prelude::*;
use valence_protocol::ItemStack;

use super::Metadata;
use crate::define_and_register_components;

// Example usage:
define_and_register_components! {
    8, Item -> ItemStack
}

impl Default for Item {
    fn default() -> Self {
        Self::new(ItemStack::EMPTY)
    }
}
