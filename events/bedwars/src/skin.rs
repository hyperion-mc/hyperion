use std::borrow::Cow;

use flecs_ecs::{
    core::{Entity, EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    egress::player_join::{PlayerListActions, PlayerListEntry, PlayerListS2c},
    net::{Compose, ConnectionId, DataBundle},
    simulation::{event, skin::PlayerSkin},
    storage::EventQueue,
    uuid::Uuid,
    valence_ident::ident,
    valence_protocol,
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

    // todo: in future, do not clone
    let property = valence_protocol::profile::Property {
        name: "textures".to_string(),
        value: skin.textures,
        signature: Some(skin.signature),
    };

    let property = &[property];

    // Add player back with new skin
    bundle
        .add_packet(&PlayerListS2c {
            actions: PlayerListActions::default().with_add_player(true),
            entries: Cow::Borrowed(&[PlayerListEntry {
                player_uuid: uuid,
                username: Cow::Borrowed("Player"),
                properties: Cow::Borrowed(property),
                chat_data: None,
                listed: true,
                ping: 20,
                game_mode: GameMode::Survival,
                display_name: None,
            }]),
        })
        .unwrap();

    // // Respawn player
    bundle
        .add_packet(&PlayerRespawnS2c {
            dimension_type_name: ident!("minecraft:overworld").into(),
            dimension_name: ident!("minecraft:overworld").into(),
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
