use bevy::prelude::*;
use tracing::{error, info_span};

use crate::{
    net::Compose,
    simulation::{blocks::Blocks, packet_state},
};

pub struct StatsPlugin;

impl Plugin for StatsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (global_update, load_pending));
        app.add_observer(player_join_world);
        app.add_observer(player_leave_world);
    }
}

fn global_update(mut compose: ResMut<'_, Compose>) {
    let global = compose.global_mut();

    global.tick += 1;
}

pub fn player_join_world(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    compose: Res<'_, Compose>,
) {
    compose
        .global()
        .player_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

// TODO: Is this ran when entity is despawned?
pub fn player_leave_world(
    trigger: Trigger<'_, OnRemove, packet_state::Play>,
    compose: Res<'_, Compose>,
) {
    compose
        .global()
        .player_count
        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

fn load_pending(mut blocks: ResMut<'_, Blocks>) {
    blocks.load_pending();
}
