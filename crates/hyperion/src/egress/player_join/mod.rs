use std::{borrow::Cow, collections::BTreeSet, ops::Index};

use anyhow::Context;
use flecs_ecs::prelude::*;
use glam::DVec3;
use hyperion_crafting::{Action, CraftingRegistry, RecipeBookState};
use hyperion_utils::EntityExt;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tracing::{info, instrument, warn};
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
    egress::metadata::show_all,
    net::{Compose, ConnectionId, DataBundle},
    simulation::{
        Comms, Name, Position, Uuid, Yaw,
        command::{Command, ROOT_COMMAND, get_command_packet},
        metadata::{MetadataChanges, entity::EntityFlags},
        skin::PlayerSkin,
        util::registry_codec_raw,
    },
    util::{SendableQuery, SendableRef},
};

#[expect(
    clippy::too_many_arguments,
    reason = "todo: we should refactor at some point"
)]
#[instrument(skip_all, fields(name = name))]
pub fn player_join_world(
    entity: &EntityView<'_>,
    compose: &Compose,
    uuid: uuid::Uuid,
    name: &str,
    io: ConnectionId,
    position: &Position,
    yaw: &Yaw,
    pitch: &Pitch,
    world: &WorldRef<'_>,
    skin: &PlayerSkin,
    system: EntityView<'_>,
    root_command: Entity,
    query: &Query<(
        &Uuid,
        &Name,
        &Position,
        &Yaw,
        &Pitch,
        &PlayerSkin,
        &EntityFlags,
    )>,
    crafting_registry: &CraftingRegistry,
    config: &Config,
) -> anyhow::Result<()> {
    static CACHED_DATA: once_cell::sync::OnceCell<bytes::Bytes> = once_cell::sync::OnceCell::new();

    let mut bundle = DataBundle::new(compose, system);

    let id = entity.minecraft_id();

    entity.set(MovementTracking {
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

    bundle
        .add_packet(&pkt)
        .context("failed to send player spawn packet")?;

    let center_chunk = position.to_chunk();

    let pkt = play::ChunkRenderDistanceCenterS2c {
        chunk_x: VarInt(i32::from(center_chunk.x)),
        chunk_z: VarInt(i32::from(center_chunk.y)),
    };

    bundle.add_packet(&pkt)?;

    let pkt = play::PlayerSpawnPositionS2c {
        position: position.as_dvec3().into(),
        angle: **yaw,
    };

    bundle.add_packet(&pkt)?;

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
            generate_cached_packet_bytes(&mut encoder, crafting_registry).unwrap();

            let bytes = encoder.take();
            bytes.freeze()
        })
        .clone();

    bundle.add_raw(&cached_data);

    let text = play::GameMessageS2c {
        chat: format!("{name} joined the world").into_cow_text(),
        overlay: false,
    };

    compose
        .broadcast(&text, system)
        .send()
        .context("failed to send player join message")?;

    bundle.add_packet(&play::PlayerPositionLookS2c {
        position: position.as_dvec3(),
        yaw: **yaw,
        pitch: **pitch,
        flags: PlayerPositionLookFlags::default(),
        teleport_id: 1.into(),
    })?;

    let mut entries = Vec::new();
    let mut all_player_names = Vec::new();

    let count = query.iter_stage(world).count();

    info!("sending skins for {count} players");

    {
        let scope = tracing::info_span!("generating_skins");
        let _enter = scope.enter();
        query
            .iter_stage(world)
            .each(|(uuid, name, _, _, _, _skin, _)| {
                // todo: in future, do not clone

                let entry = PlayerListEntry {
                    player_uuid: uuid.0,
                    username: name.to_string().into(),
                    // todo: eliminate alloc
                    properties: Cow::Owned(vec![]),
                    chat_data: None,
                    listed: true,
                    ping: 20,
                    game_mode: GameMode::Creative,
                    display_name: Some(name.to_string().into_cow_text()),
                };

                entries.push(entry);
                all_player_names.push(name.to_string());
            });
    }

    let all_player_names = all_player_names.iter().map(String::as_str).collect();

    let actions = PlayerListActions::default()
        .with_add_player(true)
        .with_update_listed(true)
        .with_update_display_name(true);

    {
        let scope = tracing::info_span!("unicasting_player_list");
        let _enter = scope.enter();
        bundle.add_packet(&PlayerListS2c {
            actions,
            entries: Cow::Owned(entries),
        })?;
    }

    {
        let scope = tracing::info_span!("sending_player_spawns");
        let _enter = scope.enter();

        // todo(Indra): this is a bit awkward.
        // todo: could also be helped by denoting some packets as infallible for serialization
        let mut query_errors = Vec::new();

        let mut metadata = MetadataChanges::default();

        query
            .iter_stage(world)
            .each_iter(|it, idx, (uuid, _, position, yaw, pitch, _, flags)| {
                let mut result = || {
                    let query_entity = it.entity(idx);

                    if entity.id() == query_entity.id() {
                        return anyhow::Ok(());
                    }

                    let pkt = play::PlayerSpawnS2c {
                        entity_id: VarInt(query_entity.minecraft_id()),
                        player_uuid: uuid.0,
                        position: position.as_dvec3(),
                        yaw: ByteAngle::from_degrees(**yaw),
                        pitch: ByteAngle::from_degrees(**pitch),
                    };

                    bundle
                        .add_packet(&pkt)
                        .context("failed to send player spawn packet")?;

                    let show_all = show_all(query_entity.minecraft_id());
                    bundle
                        .add_packet(show_all.borrow_packet())
                        .context("failed to send player spawn packet")?;

                    metadata.encode(*flags);

                    Ok(())
                };

                if let Err(e) = result() {
                    query_errors.push(e);
                }
            });

        if !query_errors.is_empty() {
            return Err(anyhow::anyhow!(
                "failed to send player spawn packets: {query_errors:?}"
            ));
        }
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
        player_uuid: uuid,
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
    compose
        .broadcast(&pkt, system)
        .send()
        .context("failed to send player list packet")?;
    bundle
        .add_packet(&pkt)
        .context("failed to send player list packet")?;

    let player_name = vec![name];

    compose
        .broadcast(
            &play::TeamS2c {
                team_name: "no_tag",
                mode: Mode::AddEntities {
                    entities: player_name,
                },
            },
            system,
        )
        .exclude(io)
        .send()
        .context("failed to send team packet")?;

    let current_entity_id = VarInt(entity.minecraft_id());

    let spawn_player = play::PlayerSpawnS2c {
        entity_id: current_entity_id,
        player_uuid: uuid,
        position: position.as_dvec3(),
        yaw: ByteAngle::from_degrees(**yaw),
        pitch: ByteAngle::from_degrees(**pitch),
    };
    compose
        .broadcast(&spawn_player, system)
        .exclude(io)
        .send()
        .context("failed to send player spawn packet")?;

    let show_all = show_all(entity.minecraft_id());
    compose
        .broadcast(show_all.borrow_packet(), system)
        .send()
        .context("failed to send show all packet")?;

    bundle
        .add_packet(&play::TeamS2c {
            team_name: "no_tag",
            mode: Mode::AddEntities {
                entities: all_player_names,
            },
        })
        .context("failed to send team packet")?;

    let command_packet = get_command_packet(world, root_command, Some(**entity));

    bundle.add_packet(&command_packet)?;

    bundle.unicast(io)?;

    info!("{name} joined the world");

    Ok(())
}

fn send_sync_tags(encoder: &mut PacketEncoder) -> anyhow::Result<()> {
    let bytes = include_bytes!("data/tags.json");

    let groups = serde_json::from_slice(bytes)?;

    let pkt = play::SynchronizeTagsS2c { groups };

    encoder.append_packet(&pkt)?;

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
    let brand = b"discord: andrewgazelka";
    let brand_len = u8::try_from(brand.len()).context("brand length too long to fit in u8")?;
    buf.push(brand_len).unwrap();
    buf.extend_from_slice(brand).unwrap();

    let bytes = RawBytes::from(buf.as_slice());

    let brand = play::CustomPayloadS2c {
        channel: ident!("minecraft:brand").into(),
        data: bytes.into(),
    };

    encoder.append_packet(&brand)?;

    encoder.append_packet(&play::TeamS2c {
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
    })?;

    if let Some(pkt) = crafting_registry.packet() {
        encoder.append_packet(&pkt)?;
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

    encoder.append_packet(&pkt)?;

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
pub struct PlayerJoinModule;

#[derive(Component)]
pub struct RayonWorldStages {
    stages: Vec<SendableRef<'static>>,
}

impl Index<usize> for RayonWorldStages {
    type Output = WorldRef<'static>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.stages[index].0
    }
}

impl Module for PlayerJoinModule {
    fn module(world: &World) {
        let query = world.new_query::<(
            &Uuid,
            &Name,
            &Position,
            &Yaw,
            &Pitch,
            &PlayerSkin,
            &EntityFlags,
        )>();

        let query = SendableQuery(query);

        let rayon_threads = rayon::current_num_threads();

        #[expect(
            clippy::unwrap_used,
            reason = "realistically, this should never fail; 2^31 is very large"
        )]
        let rayon_threads = i32::try_from(rayon_threads).unwrap();

        let stages = (0..rayon_threads)
            // SAFETY: promoting world to static lifetime, system won't outlive world
            .map(|i| unsafe { std::mem::transmute(world.stage(i)) })
            .map(SendableRef)
            .collect::<Vec<_>>();

        world.component::<RayonWorldStages>();
        world.set(RayonWorldStages { stages });

        let root_command = world.entity().set(Command::ROOT);

        #[expect(
            clippy::unwrap_used,
            reason = "this is only called once on startup. We mostly care about crashing during \
                      server execution"
        )]
        ROOT_COMMAND.set(root_command.id()).unwrap();

        let root_command = root_command.id();

        system!(
            "player_joins",
            world,
            &Comms($),
            &Compose($),
            &CraftingRegistry($),
            &Config($),
            &RayonWorldStages($),
        )
        .kind::<flecs::pipeline::PreUpdate>()
        .each_iter(
            move |it, _, (comms, compose, crafting_registry, config, stages)| {
                let span = tracing::info_span!("joins");
                let _enter = span.enter();

                let system = it.system().id();

                let mut skins = Vec::new();

                while let Ok(Some((entity, skin))) = comms.skins_rx.try_recv() {
                    skins.push((entity, skin.clone()));
                }

                // todo: par_iter but bugs...
                // for (entity, skin) in skins {
                skins.into_par_iter().for_each(|(entity, skin)| {
                    // if we are not in rayon context that means we are in a single-threaded context and 0 will work
                    let idx = rayon::current_thread_index().unwrap_or(0);
                    let world = &stages[idx];

                    let system = system.entity_view(world);

                    if !world.is_alive(entity) {
                        return;
                    }

                    let entity = world.entity_from_id(entity);

                    entity.get::<(&Uuid, &Name, &Position, &Yaw, &Pitch, &ConnectionId)>(
                        |(uuid, name, position, yaw, pitch, &stream_id)| {
                            let query = &query;
                            let query = &query.0;
                            entity.set_name(name);

                            // if we get an error joining, we should kick the player
                            if let Err(e) = player_join_world(
                                &entity,
                                compose,
                                uuid.0,
                                name,
                                stream_id,
                                position,
                                yaw,
                                pitch,
                                world,
                                &skin,
                                system,
                                root_command,
                                query,
                                crafting_registry,
                                config,
                            ) {
                                warn!("player_join_world error: {e:?}");
                                compose.io_buf().shutdown(stream_id, world);
                            }
                        },
                    );

                    let entity = world.entity_from_id(entity);
                    entity.set(skin);

                    entity.add_enum(PacketState::Play);
                });
            },
        );
    }
}
