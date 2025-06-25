use bevy::prelude::*;
use hyperion::{
    net::Compose,
    simulation::{Uuid, metadata::entity::EntityFlags},
};
use tracing::error;
use valence_protocol::packets::play::{self, player_list_s2c::PlayerListActions};
use valence_server::GameMode;

pub struct VanishPlugin;

#[derive(Default, Component, Debug)]
pub struct Vanished(bool);

impl Vanished {
    #[must_use]
    pub const fn new(is_vanished: bool) -> Self {
        Self(is_vanished)
    }

    #[must_use]
    pub const fn is_vanished(&self) -> bool {
        self.0
    }
}

fn update_vanish(
    trigger: Trigger<'_, OnInsert, Vanished>,
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (&Vanished, &Uuid, &mut EntityFlags)>,
) {
    let (vanished, uuid, mut flags) = match query.get_mut(trigger.target()) {
        Ok(data) => data,
        Err(e) => {
            error!("failed to update vanish: query failed: {e}");
            return;
        }
    };

    if vanished.is_vanished() {
        // Remove from player list and make them invisible
        let remove_packet = play::PlayerListS2c {
            actions: PlayerListActions::new()
                .with_update_listed(true)
                .with_update_game_mode(true),
            entries: vec![play::player_list_s2c::PlayerListEntry {
                player_uuid: uuid.0,
                listed: false,
                game_mode: GameMode::Survival,
                ..Default::default()
            }]
            .into(),
        };
        compose.broadcast(&remove_packet).send().unwrap();

        // Set entity flags to make them invisible
        *flags |= EntityFlags::INVISIBLE;
    } else {
        // Add back to player list and make them visible
        let add_packet = play::PlayerListS2c {
            actions: PlayerListActions::new()
                .with_update_listed(true)
                .with_update_game_mode(true),
            entries: vec![play::player_list_s2c::PlayerListEntry {
                player_uuid: uuid.0,
                listed: true,
                game_mode: GameMode::Survival,
                ..Default::default()
            }]
            .into(),
        };
        compose.broadcast(&add_packet).send().unwrap();

        // Clear invisible flag
        *flags &= !EntityFlags::INVISIBLE;
    }
}

impl Plugin for VanishPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(update_vanish);
    }
}
