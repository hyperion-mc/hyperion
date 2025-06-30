use std::borrow::Cow;

use bevy::prelude::*;
use hyperion::{
    egress::player_join::{PlayerListActions, PlayerListEntry, PlayerListS2c},
    net::{Compose, ConnectionId, DataBundle},
    simulation::event,
    valence_ident::ident,
};
use hyperion_utils::EntityExt;
use tracing::error;
use valence_bytes::Utf8Bytes;
use valence_protocol::{
    GameMode, VarInt,
    game_mode::OptGameMode,
    packets::play::{EntitiesDestroyS2c, PlayerRemoveS2c, PlayerRespawnS2c},
};

pub struct SkinPlugin;

impl Plugin for SkinPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, on_set_skin);
    }
}

fn on_set_skin(
    mut events: EventReader<'_, '_, event::SetSkin>,
    compose: Res<'_, Compose>,
    query: Query<'_, '_, (&ConnectionId, &hyperion::simulation::Uuid)>,
) {
    for event in events.read() {
        let (&connection_id, uuid) = match query.get(event.by) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to set skin: query failed: {e}");
                continue;
            }
        };

        let minecraft_id = event.by.minecraft_id();
        let mut bundle = DataBundle::new(&compose);
        // Remove player info
        bundle
            .add_packet(&PlayerRemoveS2c {
                uuids: Cow::Borrowed(&[**uuid]),
            })
            .unwrap();

        // Destroy player entity
        bundle
            .add_packet(&EntitiesDestroyS2c {
                entity_ids: Cow::Borrowed(&[VarInt(minecraft_id)]),
            })
            .unwrap();

        // todo: in future, do not clone
        let property = valence_protocol::profile::Property::<Utf8Bytes> {
            name: "textures".into(),
            value: event.skin.textures.clone().into(),
            signature: Some(event.skin.signature.clone().into()),
        };

        let property = &[property];

        // Add player back with new skin
        bundle
            .add_packet(&PlayerListS2c {
                actions: PlayerListActions::default().with_add_player(true),
                entries: Cow::Borrowed(&[PlayerListEntry {
                    player_uuid: **uuid,
                    username: "Player".into(),
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

        bundle.unicast(connection_id).unwrap();
    }
}
