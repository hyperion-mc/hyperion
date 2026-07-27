use flecs_ecs::{core::World, prelude::*};
use hyperion::{
    hyperion_minecraft_proto::{
        Uuid as ProtoUuid,
        generated::packet_id::play::clientbound::PacketId,
        packets::{
            play::player::{PlayerInfoActions, PlayerInfoEntry, PlayerInfoUpdate},
            play_login::GameType,
        },
    },
    net::{Compose, ConnectionId, protocol::Clientbound},
    simulation::{Uuid, metadata::entity::EntityFlags},
};

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
            &Compose,
            &ConnectionId,
            &Vanished,
            &Uuid,
        )
        .kind(id::<flecs::pipeline::PreStore>())
        .each_iter(move |it, row, (compose, _connection_id, vanished, uuid)| {
            let entity = it.entity(row);
            let world = it.world();

            let listed = !vanished.is_vanished();

            // `UPDATE_LISTED` is what hides the row; the game mode rides along
            // because a client that has never seen this player needs one and
            // the entry costs a byte either way.
            let packet = PlayerInfoUpdate {
                actions: PlayerInfoActions::UPDATE_LISTED
                    .union(PlayerInfoActions::UPDATE_GAME_MODE),
                entries: vec![PlayerInfoEntry {
                    profile_id: ProtoUuid(uuid.0.as_u128()),
                    listed,
                    game_mode: GameType::Survival,
                    ..PlayerInfoEntry::default()
                }],
            };
            compose
                .broadcast(Clientbound::new(
                    PacketId::PlayerInfoUpdate.to_raw(),
                    &packet,
                ))
                .send()
                .unwrap();

            let flags = if listed {
                EntityFlags::default()
            } else {
                EntityFlags::INVISIBLE
            };
            entity.entity_view(world).set(flags);
        });
    }
}
