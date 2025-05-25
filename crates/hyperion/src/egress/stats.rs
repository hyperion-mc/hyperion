use bevy::prelude::*;
use tracing::{error, info_span};

use crate::{
    net::Compose,
    simulation::{PacketState, blocks::Blocks},
};

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (global_update, load_pending));
    }
}

fn global_update(
    mut compose: ResMut<'_, Compose>,
    mut players: Query<'_, '_, (), With<PacketState>>,
) {
    let global = compose.global_mut();

    global.tick += 1;

    // let player_count = compose.global().shared.player_count.load(std::sync::atomic::Ordering::Relaxed);
    let player_count = players.count();

    let Ok(player_count) = usize::try_from(player_count) else {
        // should never be a negative number. this is just in case.
        error!("failed to convert player count to usize. Was {player_count}");
        return;
    };

    *global.player_count.get_mut() = player_count;
}

fn load_pending(mut blocks: ResMut<'_, Blocks>) {
    blocks.load_pending();
}
