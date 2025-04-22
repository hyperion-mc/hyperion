use flecs_ecs::{core::World, prelude::*};
use hyperion::{
    net::{Compose, ConnectionId},
    simulation::{Uuid, metadata::entity::EntityFlags},
};
use valence_protocol::packets::play::{self, player_list_s2c::PlayerListActions};
use valence_server::GameMode;

#[derive(Component)]
pub struct VanishModule;

#[derive(Default, Component, Debug)]
pub struct Vanished(pub bool);

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

impl Module for VanishModule {
    fn module(world: &World) {
        world.component::<Vanished>();

        system!(
            "vanish_sync",
            world,
            &Compose($),
            &ConnectionId,
            &Vanished,
            &Uuid,
        )
        .kind::<flecs::pipeline::PreStore>()
        .each_iter(move |it, row, (compose, _connection_id, vanished, uuid)| {
            let entity = it.entity(row);
            let system = it.system();
            let world = it.world();

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
                compose.broadcast(&remove_packet, system).send().unwrap();

                // Set entity flags to make them invisible
                let flags = EntityFlags::INVISIBLE;
                entity.entity_view(world).set(flags);
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
                compose.broadcast(&add_packet, system).send().unwrap();

                // Clear invisible flag
                let flags = EntityFlags::default();
                entity.entity_view(world).set(flags);
            }
        });
    }
}
