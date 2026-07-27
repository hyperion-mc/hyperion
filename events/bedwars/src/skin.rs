use std::borrow::Cow;

use flecs_ecs::{
    core::{Entity, EntityViewGet, SystemAPI, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    egress::player_join::{PlayerInfoActions, PlayerList, PlayerListEntry, SkinProperty},
    net::{Compose, ConnectionId, DataBundle},
    simulation::{event, skin::PlayerSkin},
    storage::EventQueue,
    uuid::Uuid,
    valence_ident::ident,
    valence_protocol::{
        GameMode, VarInt,
        game_mode::OptGameMode,
        packets::play::{EntitiesDestroyS2c, PlayerRemoveS2c, PlayerRespawnS2c},
    },
};
use hyperion_utils::EntityExt;
use tracing::debug;

#[derive(Component)]
pub struct SkinModule;

impl Module for SkinModule {
    fn module(world: &World) {
        system!("set_skin", world, &mut EventQueue<event::SetSkin>, &Compose).each_iter(
            |it, _, (event_queue, compose)| {
                let world = it.world();
                for event in event_queue.drain() {
                    debug!("got {event:?}");
                    event
                        .by
                        .entity_view(world)
                        .get::<(&ConnectionId, &hyperion::simulation::Uuid)>(|(io, uuid)| {
                            on_set_skin(event.by, compose, uuid.0, event.skin, *io);
                        });
                }
            },
        );
    }
}

fn on_set_skin(id: Entity, compose: &Compose, uuid: Uuid, skin: PlayerSkin, io: ConnectionId) {
    let minecraft_id = id.minecraft_id();
    let mut bundle = DataBundle::new(compose);
    // Remove player info
    bundle
        .add_packet(&PlayerRemoveS2c {
            uuids: Cow::Borrowed(&[uuid]),
        })
        .unwrap();

    // Destroy player entity
    bundle
        .add_packet(&EntitiesDestroyS2c {
            entity_ids: Cow::Borrowed(&[VarInt(minecraft_id)]),
        })
        .unwrap();

    // Add player back with new skin. Only `ADD_PLAYER` is set, so the entry's
    // other fields are not on the wire and are left at their defaults.
    bundle
        .add_packet(&PlayerList {
            actions: PlayerInfoActions::ADD_PLAYER,
            entries: vec![PlayerListEntry {
                uuid,
                username: "Player".to_owned(),
                properties: vec![SkinProperty {
                    name: "textures".to_owned(),
                    value: skin.textures,
                    signature: Some(skin.signature),
                }],
                ..PlayerListEntry::default()
            }],
        })
        .unwrap();

    // // Respawn player
    bundle
        .add_packet(&PlayerRespawnS2c {
            dimension_type_name: ident!("minecraft:overworld"),
            dimension_name: ident!("minecraft:overworld"),
            hashed_seed: 0,
            game_mode: GameMode::Survival,
            previous_game_mode: OptGameMode::default(),
            is_debug: false,
            is_flat: false,
            copy_metadata: false,
            last_death_location: None,
            portal_cooldown: VarInt::default(),
        })
        .unwrap();

    bundle.unicast(io).unwrap();
}
