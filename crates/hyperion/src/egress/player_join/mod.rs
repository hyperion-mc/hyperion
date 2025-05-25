use std::{borrow::Cow, collections::BTreeSet, ops::Index};

use anyhow::Context;
use bevy::{ecs::query::QueryEntityError, prelude::*};
use glam::DVec3;
use hyperion_crafting::{Action, CraftingRegistry, RecipeBookState};
use hyperion_utils::EntityExt;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tracing::{error, info, instrument, trace, warn};
use valence_protocol::{
    ByteAngle, GameMode, Ident, PacketEncoder, RawBytes, VarInt, Velocity,
    game_mode::OptGameMode,
    ident,
    packets::play::{
        self, GameJoinS2c,
        player_position_look_s2c::PlayerPositionLookFlags,
        team_s2c::{CollisionRule, Mode, NameTagVisibility, TeamColor, TeamFlags},
    },
};
use valence_registry::{BiomeRegistry, RegistryCodec};
use valence_server::entity::EntityKind;
use valence_text::IntoText;

use crate::simulation::{MovementTracking, PacketState, Pitch};

mod list;
pub use list::*;

use crate::{
    config::Config,
    // egress::metadata::show_all,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        Comms,
        Name,
        Position,
        Uuid,
        Yaw,
        packet_state,
        // command::{Command, ROOT_COMMAND, get_command_packet},
        // metadata::{MetadataChanges, entity::EntityFlags},
        skin::PlayerSkin,
        util::registry_codec_raw,
    },
};

#[expect(
    clippy::too_many_arguments,
    reason = "todo: we should refactor at some point"
)]
#[instrument(skip_all)]
pub fn player_join_world(
    trigger: Trigger<'_, OnAdd, (Position, PlayerSkin)>,
    compose: Res<'_, Compose>,
    crafting_registry: Res<'_, CraftingRegistry>,
    config: Res<'_, Config>,
    mut commands: Commands<'_, '_>,
    target_query: Query<
        '_,
        '_,
        (
            &Uuid,
            &Name,
            &ConnectionId,
            &Position,
            &Yaw,
            &Pitch,
            &PlayerSkin,
        ),
    >,
    others_query: Query<
        '_,
        '_,
        (
            Entity,
            &Uuid,
            &Name,
            &Position,
            &Yaw,
            &Pitch,
            &PlayerSkin,
            // &EntityFlags,
        ),
    >,
) {
    // TODO: Assert that player is in play state
    static CACHED_DATA: once_cell::sync::OnceCell<bytes::Bytes> = once_cell::sync::OnceCell::new();

    let mut bundle = DataBundle::new(&compose);

    let entity_id = trigger.target();
    let id = entity_id.minecraft_id();

    // TODO: skin may not exist yet
    let (uuid, name, &connection_id, position, yaw, pitch, skin) = match target_query.get(entity_id)
    {
        Ok(components) => components,
        Err(QueryEntityError::QueryDoesNotMatch(..)) => {
            trace!(
                "player_join_world failed because player is missing components (this is expected \
                 if either Position or PlayerSkin has not been set yet"
            );
            return;
        }
        Err(e) => {
            error!("player_join_world failed: {e}");
            return;
        }
    };

    let mut entity = commands.entity(entity_id);

    entity.insert(MovementTracking {
        received_movement_packets: 0,
        last_tick_flying: false,
        last_tick_position: **position,
        fall_start_y: position.y,
        server_velocity: DVec3::ZERO,
        sprinting: false,
        was_on_ground: false,
    });

    let registry_codec = registry_codec_raw();
    let codec = RegistryCodec::default();

    let dimension_names: BTreeSet<Ident<Cow<'_, str>>> = codec
        .registry(BiomeRegistry::KEY)
        .iter()
        .map(|value| value.name.as_str_ident().into())
        .collect();

    let dimension_name = ident!("overworld");
    // let dimension_name: Ident<Cow<str>> = chunk_layer.dimension_type_name().into();

    let pkt = GameJoinS2c {
        entity_id: id,
        is_hardcore: false,
        dimension_names: Cow::Owned(dimension_names),
        registry_codec: Cow::Borrowed(registry_codec),
        max_players: config.max_players.into(),
        view_distance: VarInt(i32::from(config.view_distance)),
        simulation_distance: config.simulation_distance.into(),
        reduced_debug_info: false,
        enable_respawn_screen: false,
        dimension_name: dimension_name.into(),
        hashed_seed: 0,
        game_mode: GameMode::Survival,
        is_flat: false,
        last_death_location: None,
        portal_cooldown: 60.into(),
        previous_game_mode: OptGameMode(Some(GameMode::Survival)),
        dimension_type_name: ident!("minecraft:overworld").into(),
        is_debug: false,
    };

    bundle.add_packet(&pkt).unwrap();

    let center_chunk = position.to_chunk();

    let pkt = play::ChunkRenderDistanceCenterS2c {
        chunk_x: VarInt(i32::from(center_chunk.x)),
        chunk_z: VarInt(i32::from(center_chunk.y)),
    };

    bundle.add_packet(&pkt).unwrap();

    let pkt = play::PlayerSpawnPositionS2c {
        position: position.as_dvec3().into(),
        angle: **yaw,
    };

    bundle.add_packet(&pkt).unwrap();

    let cached_data = CACHED_DATA
        .get_or_init(|| {
            let compression_level = compose.global().shared.compression_threshold;
            let mut encoder = PacketEncoder::new();
            encoder.set_compression(compression_level);

            info!(
                "caching world data for new players with compression level {compression_level:?}"
            );

            #[expect(
                clippy::unwrap_used,
                reason = "this is only called once on startup; it should be fine. we mostly care \
                          about crashing during server execution"
            )]
            generate_cached_packet_bytes(&mut encoder, crafting_registry.into_inner()).unwrap();

            let bytes = encoder.take();
            bytes.freeze()
        })
        .clone();

    bundle.add_raw(&cached_data);

    let text = play::GameMessageS2c {
        chat: format!("{name} joined the world").into_cow_text(),
        overlay: false,
    };

    compose.broadcast(&text).send().unwrap();

    bundle
        .add_packet(&play::PlayerPositionLookS2c {
            position: position.as_dvec3(),
            yaw: **yaw,
            pitch: **pitch,
            flags: PlayerPositionLookFlags::default(),
            teleport_id: 1.into(),
        })
        .unwrap();
    let mut entries = Vec::new();
    let mut all_player_names = Vec::new();

    for (current_entity, uuid, name, position, yaw, pitch, skin) in others_query {
        if entity_id == current_entity {
            continue;
        }

        // Update player list entries
        let entry = PlayerListEntry {
            player_uuid: uuid.0,
            username: name.to_string().into(),
            properties: Cow::Owned(Vec::new()),
            chat_data: None,
            listed: true,
            ping: 20,
            game_mode: GameMode::Creative,
            display_name: Some(name.to_string().into_cow_text()),
        };

        entries.push(entry);
        all_player_names.push(name.to_string());

        // Spawn the current entity for the player that is joining
        let pkt = play::PlayerSpawnS2c {
            entity_id: VarInt(current_entity.minecraft_id()),
            player_uuid: uuid.0,
            position: position.as_dvec3(),
            yaw: ByteAngle::from_degrees(**yaw),
            pitch: ByteAngle::from_degrees(**pitch),
        };

        bundle.add_packet(&pkt).unwrap();

        //         let show_all = show_all(current_entity.minecraft_id());
        //         bundle
        //             .add_packet(show_all.borrow_packet())
        //             .unwrap();

        //         metadata.encode(*flags);
    }

    let all_player_names = all_player_names.iter().map(String::as_str).collect();

    let actions = PlayerListActions::default()
        .with_add_player(true)
        .with_update_listed(true)
        .with_update_display_name(true);

    {
        let scope = tracing::info_span!("unicasting_player_list");
        let _enter = scope.enter();
        bundle
            .add_packet(&PlayerListS2c {
                actions,
                entries: Cow::Owned(entries),
            })
            .unwrap();
    }

    let PlayerSkin {
        textures,
        signature,
    } = skin.clone();

    // todo: in future, do not clone
    let property = valence_protocol::profile::Property {
        name: "textures".to_string(),
        value: textures,
        signature: Some(signature),
    };

    let property = &[property];

    let singleton_entry = &[PlayerListEntry {
        player_uuid: **uuid,
        username: Cow::Borrowed(name),
        properties: Cow::Borrowed(property),
        chat_data: None,
        listed: true,
        ping: 20,
        game_mode: GameMode::Survival,
        display_name: Some(name.to_string().into_cow_text()),
    }];

    let pkt = PlayerListS2c {
        actions,
        entries: Cow::Borrowed(singleton_entry),
    };

    // todo: fix broadcasting on first tick; and this duplication can be removed!
    compose.broadcast(&pkt).send().unwrap();
    bundle.add_packet(&pkt).unwrap();

    let player_name: Vec<&str> = vec![&**name];

    compose
        .broadcast(&play::TeamS2c {
            team_name: "no_tag",
            mode: Mode::AddEntities {
                entities: player_name,
            },
        })
        .exclude(connection_id)
        .send()
        .unwrap();

    let current_entity_id = VarInt(entity_id.minecraft_id());

    let spawn_player = play::PlayerSpawnS2c {
        entity_id: current_entity_id,
        player_uuid: **uuid,
        position: position.as_dvec3(),
        yaw: ByteAngle::from_degrees(**yaw),
        pitch: ByteAngle::from_degrees(**pitch),
    };
    compose
        .broadcast(&spawn_player)
        .exclude(connection_id)
        .send()
        .unwrap();

    //     let show_all = show_all(entity_id.minecraft_id());
    //     compose
    //         .broadcast(show_all.borrow_packet())
    //         .send()
    //         .unwrap();

    bundle
        .add_packet(&play::TeamS2c {
            team_name: "no_tag",
            mode: Mode::AddEntities {
                entities: all_player_names,
            },
        })
        .unwrap();

    //     let command_packet = get_command_packet(root_command, Some(**entity));
    //
    //     bundle.add_packet(&command_packet).unwrap();

    bundle.unicast(connection_id).unwrap();

    info!("{name} joined the world");
}

fn send_sync_tags(encoder: &mut PacketEncoder) -> anyhow::Result<()> {
    let bytes = include_bytes!("data/tags.json");

    let groups = serde_json::from_slice(bytes)?;

    let pkt = play::SynchronizeTagsS2c { groups };

    encoder
        .append_packet(&pkt)
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

#[expect(
    clippy::unwrap_used,
    reason = "this is only called once on startup; it should be fine. we mostly care about \
              crashing during server execution"
)]
fn generate_cached_packet_bytes(
    encoder: &mut PacketEncoder,
    crafting_registry: &CraftingRegistry,
) -> anyhow::Result<()> {
    send_sync_tags(encoder)?;

    let mut buf: heapless::Vec<u8, 32> = heapless::Vec::new();
    let brand = b"hyperion";
    let brand_len = u8::try_from(brand.len()).context("brand length too long to fit in u8")?;
    buf.push(brand_len).unwrap();
    buf.extend_from_slice(brand).unwrap();

    let bytes = RawBytes::from(buf.as_slice());

    let brand = play::CustomPayloadS2c {
        channel: ident!("minecraft:brand").into(),
        data: bytes.into(),
    };

    encoder
        .append_packet(&brand)
        .map_err(|e| anyhow::anyhow!(e))?;

    encoder
        .append_packet(&play::TeamS2c {
            team_name: "no_tag",
            mode: Mode::CreateTeam {
                team_display_name: Cow::default(),
                friendly_flags: TeamFlags::default(),
                name_tag_visibility: NameTagVisibility::Never,
                collision_rule: CollisionRule::Always,
                team_color: TeamColor::Black,
                team_prefix: Cow::default(),
                team_suffix: Cow::default(),
                entities: vec![],
            },
        })
        .map_err(|e| anyhow::anyhow!(e))?;

    if let Some(pkt) = crafting_registry.packet() {
        encoder
            .append_packet(&pkt)
            .map_err(|e| anyhow::anyhow!(e))?;
    }

    // unlock
    let pkt = hyperion_crafting::UnlockRecipesS2c {
        action: Action::Init,
        crafting_recipe_book: RecipeBookState::FALSE,
        smelting_recipe_book: RecipeBookState::FALSE,
        blast_furnace_recipe_book: RecipeBookState::FALSE,
        smoker_recipe_book: RecipeBookState::FALSE,
        recipe_ids_1: vec!["hyperion:what".to_string()],
        recipe_ids_2: vec!["hyperion:what".to_string()],
    };

    encoder
        .append_packet(&pkt)
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

#[tracing::instrument(skip_all)]
pub fn spawn_entity_packet(
    id: Entity,
    kind: EntityKind,
    uuid: Uuid,
    yaw: &Yaw,
    pitch: &Pitch,
    position: &Position,
) -> play::EntitySpawnS2c {
    info!("spawning entity");

    let entity_id = VarInt(id.minecraft_id());

    play::EntitySpawnS2c {
        entity_id,
        object_uuid: *uuid,
        kind: VarInt(kind.get()),
        position: position.as_dvec3(),
        yaw: ByteAngle::from_degrees(**yaw),
        pitch: ByteAngle::from_degrees(**pitch),
        head_yaw: ByteAngle::from_degrees(**yaw), // todo: unsure if this is correct
        data: VarInt::default(),
        velocity: Velocity([0; 3]),
    }
}

#[derive(Component)]
pub struct PlayerJoinPlugin;

impl Plugin for PlayerJoinPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(player_join_world);
        // let world = app.world_mut();

        // world.spawn()
        // let root_command = world.spawn(Command::ROOT);
    }
}
