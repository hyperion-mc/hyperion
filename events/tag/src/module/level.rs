use bevy::prelude::*;
use hyperion::simulation::{Xp, packet_state};
use hyperion_inventory::PlayerInventory;
use hyperion_rank_tree::{Class, Handles, Team};
use tracing::error;

use crate::MainBlockCount;

#[derive(Component, Default, Copy, Clone, Debug)]
pub struct UpgradedTo {
    pub value: u8,
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert(UpgradedTo::default());
}

fn update_level(
    trigger: Trigger<'_, OnInsert, Xp>,
    mut query: Query<
        '_,
        '_,
        (
            &Xp,
            &UpgradedTo,
            &Class,
            &Team,
            &MainBlockCount,
            &mut PlayerInventory,
        ),
    >,
    handles: Res<'_, Handles>,
) {
    let (xp, upgraded_to, rank, team, main_block_count, mut inventory) =
        match query.get_mut(trigger.target()) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to update level: query failed: {e}");
                return;
            }
        };

    let new_level = xp.get_visual().level;
    let level_diff = new_level - upgraded_to.value;
    rank.apply_inventory(
        *team,
        &mut inventory,
        &handles,
        **main_block_count,
        level_diff,
    );
}

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_player);
        app.add_observer(update_level);
    }
}
