use std::{cell::RefCell, sync::Arc};

use anyhow::{Context, bail};
use bytes::BytesMut;
use derive_more::Constructor;
use glam::{I16Vec2, IVec2};
use hyperion_minecraft_proto::{
    block_state,
    generated::packet_id::play::clientbound::PacketId,
    world::{
        ContainerKind, PalettedContainer,
        chunk::{ChunkData, ChunkSection, Heightmap, HeightmapKind, LevelChunkWithLight, LightData, light_mask},
    },
};
use hyperion_nerd_font::NERD_ROCKET;
use itertools::Itertools;
use libdeflater::{CompressionLvl, Compressor};
use parse::ColumnData;
use rustc_hash::FxHashSet;
use tracing::{debug, warn};
use valence_protocol::CompressionThreshold;
use valence_registry::RegistryIdx;
use valence_server::layer::chunk::Chunk;

pub mod parse;

use super::{chunk::Column, shared::WorldShared, translate};
use crate::{
    CHUNK_HEIGHT_SPAN, Scratch,
    net::{
        encoder::PacketEncoder,
        protocol::{Clientbound, registries},
    },
    runtime::AsyncRuntime,
    simulation::{blocks::loader::parse::section::Section, util::heightmap},
};

struct TasksState {
    bytes: BytesMut,
    compressor: Compressor,
    scratch: Scratch,
}

impl Default for TasksState {
    fn default() -> Self {
        Self {
            bytes: BytesMut::new(),
            compressor: Compressor::new(CompressionLvl::new(1).unwrap()),
            scratch: Scratch::default(),
        }
    }
}

thread_local! {
  static STATE: RefCell<TasksState> = RefCell::new(TasksState::default());
}

struct Message {
    position: I16Vec2,
    tx: tokio::sync::mpsc::UnboundedSender<Column>,
}

struct ChunkLoader {
    rx_load_chunk_requests: tokio::sync::mpsc::UnboundedReceiver<Message>,
    received_request: FxHashSet<I16Vec2>,
    shared: Arc<WorldShared>,
    runtime: AsyncRuntime,
}

#[derive(Constructor)]
pub struct ChunkLoaderHandle {
    tx_load_chunk_requests: tokio::sync::mpsc::UnboundedSender<Message>,
}

impl ChunkLoaderHandle {
    pub fn send(&self, position: I16Vec2, tx: tokio::sync::mpsc::UnboundedSender<Column>) {
        self.tx_load_chunk_requests
            .send(Message { position, tx })
            .unwrap();
    }
}

pub fn launch_loader(shared: Arc<WorldShared>, runtime: &AsyncRuntime) -> ChunkLoaderHandle {
    let (tx_load_chunk_requests, rx_load_chunk_requests) = tokio::sync::mpsc::unbounded_channel();

    runtime.spawn({
        let runtime = runtime.clone();
        async move {
            ChunkLoader {
                rx_load_chunk_requests,
                received_request: FxHashSet::default(),
                shared,
                runtime,
            }
            .run()
            .await;
        }
    });

    ChunkLoaderHandle {
        tx_load_chunk_requests,
    }
}

pub fn launch_empty_loader(runtime: &AsyncRuntime) -> ChunkLoaderHandle {
    let (tx_loaded_chunks, mut rx_loaded_chunks) =
        tokio::sync::mpsc::unbounded_channel::<Message>();

    runtime.spawn(async move {
        while let Some(msg) = rx_loaded_chunks.recv().await {
            let column = empty_column(msg.position);
            msg.tx.send(column).unwrap();
        }
    });

    ChunkLoaderHandle::new(tx_loaded_chunks)
}

impl ChunkLoader {
    async fn run(mut self) {
        while let Some(message) = self.rx_load_chunk_requests.recv().await {
            self.handle_load_chunk(message);
        }
    }

    fn handle_load_chunk(&mut self, message: Message) {
        let position = message.position;
        let newly_inserted = self.received_request.insert(position);

        if !newly_inserted {
            // people should already have a cached version of this chunk
            // or we are about to send it to them
            return;
        }

        let tx_load_chunks = message.tx;
        let shared = self.shared.clone();

        self.runtime.spawn(async move {
            let loaded_chunk = match load_chunk(position, &shared).await {
                Ok(loaded_chunk) => {
                    let chunk_height = loaded_chunk.data.height();
                    if chunk_height == CHUNK_HEIGHT_SPAN {
                        loaded_chunk
                    } else {
                        warn!(
                            "got a chunk that did not have the correct height at {position}, \
                             setting to empty. This can happen if a chunk was generated in an old \
                             version of Minecraft.\n\nExpected height: {CHUNK_HEIGHT_SPAN}, got \
                             {chunk_height}"
                        );
                        empty_column(position)
                    }
                }
                Err(err) => {
                    warn!("failed to load chunk {position:?}: {err}");
                    empty_column(position)
                }
            };

            let unique_blocks = loaded_chunk
                .data
                .sections
                .iter()
                .flat_map(|section| section.block_states.unique_blocks())
                .unique()
                .count();

            debug!("{NERD_ROCKET} loaded chunk {position} with {unique_blocks} unique blocks");

            tx_load_chunks.send(loaded_chunk).unwrap();
        });
    }
}

fn empty_column(position: I16Vec2) -> Column {
    // height: 24
    let unloaded = ColumnData::new_with(CHUNK_HEIGHT_SPAN, Section::empty_sky);
    let position = position.as_ivec2();

    let bytes = STATE.with_borrow_mut(|state| {
        encode_chunk_packet(&unloaded, position, state)
            .unwrap()
            .unwrap()
    });

    debug_assert_eq!(unloaded.height(), CHUNK_HEIGHT_SPAN);

    Column::new(bytes.freeze(), unloaded, position)
}

async fn load_chunk(position: I16Vec2, shared: &WorldShared) -> anyhow::Result<Column> {
    let x = position.x;
    let y = position.y;

    // todo: I do not love this heap allocation.
    let mut decompress_buf = vec![0; 1024 * 1024];

    // https://rust-lang.github.io/rust-clippy/master/index.html#/large_futures
    let Ok(region) = shared.regions.get_region_from_chunk(x, y).await else {
        // most likely the file representing the region does not exist so we will just return en empty chunk
        warn!("region file for {position} does not exist; returning empty chunk");
        return Ok(empty_column(position));
    };

    let raw_chunk = {
        // todo: note that this is likely blocking to tokio
        let x = i32::from(x);
        let y = i32::from(y);
        region
            .get_chunk(x, y, &mut decompress_buf, shared.regions.root())?
            .context("no chunk found")?
    };

    let chunk = match parse::parse_chunk(raw_chunk.data, &shared.biome_to_id) {
        Ok(chunk) => chunk,
        Err(err) => {
            bail!("failed to parse chunk {position}: {err}");
        }
    };

    STATE.with_borrow_mut(|state| {
        let position = position.as_ivec2();
        let Ok(Some(bytes)) = encode_chunk_packet(&chunk, position, state) else {
            bail!("failed to encode chunk {position:?}");
        };

        let loaded_chunk = Column::new(bytes.freeze(), chunk, position);

        Ok(loaded_chunk)
    })
}

/// The light sections a chunk has: one per chunk section plus one below the
/// world and one above, which is where light spilling in from outside lives
/// (`LevelLightEngine.getLightSectionCount`).
const LIGHT_SECTION_COUNT: usize = SECTION_COUNT + 2;

/// Chunk sections in a column, fixed by the overworld's 384-block height.
const SECTION_COUNT: usize = CHUNK_HEIGHT_SPAN as usize / 16;

/// Sky light for a section the anvil file did not store.
///
/// Full bright rather than dark: this server has no light engine, so a section
/// with no stored light would otherwise render pitch black.
const FULL_BRIGHT: [u8; 2048] = [0xff; 2048];

/// The paletted-container shapes for this protocol version.
///
/// Both are derived from a registry size, and getting either wrong moves the
/// boundary at which a container switches to the global palette, which
/// desynchronises the section blob rather than producing an error.
fn container_kinds() -> (ContainerKind, ContainerKind) {
    (
        ContainerKind::block_states(block_state::STATE_COUNT as usize),
        ContainerKind::biomes(registries::WORLDGEN_BIOME.entries.len()),
    )
}

/// Translate one stored section into the form the wire carries.
fn encode_section(
    section: &Section,
    states: ContainerKind,
    biomes: ContainerKind,
) -> anyhow::Result<ChunkSection> {
    let mut non_empty_block_count: i16 = 0;
    let mut fluid_count: i16 = 0;

    let block_states = match &section.block_states {
        // By far the common case: an untouched section is 4096 of one block,
        // and skipping the 4096-entry vector keeps chunk loading off the
        // allocator for most of a column.
        hyperion_palette::PalettedContainer::Single(raw) => {
            let state = valence_generated::block::BlockState::from_raw(*raw)
                .context("stored block state is not a 1.20.1 state")?;
            if !state.is_air() {
                non_empty_block_count = 4096;
            }
            if state.is_liquid() {
                fluid_count = 4096;
            }
            PalettedContainer::single(states, i32::try_from(translate::block_state(state))?)
        }
        container => {
            let mut values = Vec::with_capacity(states.entry_count());
            for raw in container {
                let state = valence_generated::block::BlockState::from_raw(raw)
                    .context("stored block state is not a 1.20.1 state")?;
                if !state.is_air() {
                    non_empty_block_count += 1;
                }
                // Undercounts waterlogged blocks, which vanilla would include.
                // The client reads `fluidCount` into a field it never renders
                // from; only `nonEmptyBlockCount` decides whether a section is
                // drawn, and that one is exact.
                if state.is_liquid() {
                    fluid_count += 1;
                }
                values.push(i32::try_from(translate::block_state(state))?);
            }
            // The default is the first value rather than air. Vanilla only
            // seeds a section's palette with air when it *creates* one; a
            // section read off disk gets the palette the file had. Passing
            // air here would add an eighteenth entry to a seventeen-block
            // section and push it from four bits per block to five.
            let default = values[0];
            PalettedContainer::from_values(states, default, &values)?
        }
    };

    let biome_values: Vec<i32> = (0..biomes.entry_count())
        .map(|index| i32::try_from(section.biomes.get(index).to_index()).unwrap_or(0))
        .collect();
    let biomes = PalettedContainer::from_values(biomes, biome_values[0], &biome_values)?;

    Ok(ChunkSection {
        non_empty_block_count,
        fluid_count,
        block_states,
        biomes,
    })
}

/// Build the `level_chunk_with_light` body for one column.
///
/// # Errors
/// Returns an error when the column is not the overworld's height or holds a
/// block state that is not a 1.20.1 state, both of which mean the anvil parse
/// produced something this encoder cannot describe.
pub fn build_chunk_packet(
    chunk: &ColumnData,
    location: IVec2,
) -> anyhow::Result<LevelChunkWithLight<'static>> {
    let (state_kind, biome_kind) = container_kinds();

    anyhow::ensure!(
        chunk.sections.len() == SECTION_COUNT,
        "column at {location} has {} sections, but the overworld has {SECTION_COUNT}",
        chunk.sections.len()
    );

    // A placeholder rather than a real heightmap: hyperion has no light engine
    // and nothing that needs the true surface, and a column claiming to be
    // solid to the top is what the 763 path sent too. All three client-visible
    // kinds carry it because the client leaves a kind it was not sent at zero,
    // which would put the surface at the bottom of the world.
    let placeholder = heightmap(CHUNK_HEIGHT_SPAN, CHUNK_HEIGHT_SPAN - 3)
        .into_iter()
        .map(i64::try_from)
        .try_collect::<_, Vec<i64>, _>()?;
    let heightmaps = HeightmapKind::CLIENT
        .into_iter()
        .map(|kind| Heightmap {
            kind,
            data: placeholder.clone(),
        })
        .collect();

    let mut sections = Vec::with_capacity(SECTION_COUNT);
    let mut sky_sections = Vec::with_capacity(LIGHT_SECTION_COUNT);
    let mut block_sections = Vec::new();
    let mut sky_updates = Vec::with_capacity(LIGHT_SECTION_COUNT);
    let mut block_updates = Vec::new();

    for (index, section) in chunk.sections.iter().enumerate() {
        sections.push(encode_section(section, state_kind, biome_kind)?);

        // Light section 0 is the one below the world, so a chunk section's
        // light index is one higher than its own.
        let light_index = index + 1;

        sky_sections.push(light_index);
        sky_updates.push(section.sky_light.unwrap_or(FULL_BRIGHT).to_vec());

        if let Some(block_light) = section.block_light {
            block_sections.push(light_index);
            block_updates.push(block_light.to_vec());
        }
    }

    // The section above the world, so that sky light reaches the top layer of
    // blocks from outside rather than stopping at it.
    sky_sections.push(LIGHT_SECTION_COUNT - 1);
    sky_updates.push(FULL_BRIGHT.to_vec());

    Ok(LevelChunkWithLight {
        x: location.x,
        z: location.y,
        chunk: ChunkData {
            heightmaps,
            sections,
            // Empty on purpose. A block entity carries a network id into
            // `minecraft:block_entity_type` and an NBT update tag, and both
            // are 1.20.1-shaped here: the ids are a different registry and the
            // tags predate data components. Sending them untranslated would
            // disconnect the client, so until they are ported a chest is a
            // chest-shaped block with no contents. Tracked as part of the 776
            // port rather than fixed here.
            block_entities: Vec::new(),
        },
        light: LightData {
            sky_mask: light_mask(&sky_sections),
            block_mask: light_mask(&block_sections),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky_updates,
            block_updates,
        },
    })
}

/// Frame and compress one column's packet.
///
/// The bytes are cached on the [`Column`], because a column goes to every
/// player who walks into range and building it is the expensive part.
fn encode_chunk_packet(
    chunk: &ColumnData,
    location: IVec2,
    state: &mut TasksState,
) -> anyhow::Result<Option<BytesMut>> {
    let encoder = PacketEncoder::new(CompressionThreshold::from(6));
    let packet = build_chunk_packet(chunk, location)?;

    let buf = &mut state.bytes;
    let scratch = &mut state.scratch;
    let compressor = &mut state.compressor;

    let result = encoder.append_packet(
        Clientbound::new(PacketId::LevelChunkWithLight.to_raw(), &packet),
        buf,
        scratch,
        compressor,
    )?;

    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use glam::IVec2;
    use hyperion_minecraft_proto::{Encode, Reader, Writer, block_state};
    use valence_generated::block::BlockState;
    use valence_server::layer::chunk::Chunk;

    use super::{ColumnData, SECTION_COUNT, build_chunk_packet, container_kinds};
    use crate::{CHUNK_HEIGHT_SPAN, simulation::blocks::loader::parse::section::Section};

    /// A column that carries real terrain, decoded back with the same reader a
    /// client uses.
    ///
    /// The section blob has no internal framing: a container written at the
    /// wrong bit width leaves the reader mid-section with no error until the
    /// blob runs out, so a decode that consumes exactly the bytes the encoder
    /// produced is the check that the whole column is self-consistent.
    #[test]
    fn a_built_column_decodes_as_the_client_reads_it() {
        let mut column = ColumnData::new_with(CHUNK_HEIGHT_SPAN, Section::empty_sky);
        // One uniform section and one with a handful of distinct blocks, so
        // that both the single-value and the palette path are exercised.
        column.fill_block_state_section(0, BlockState::STONE);
        for (index, state) in [
            BlockState::DIRT,
            BlockState::GRASS_BLOCK,
            BlockState::CHEST,
            BlockState::WATER,
        ]
        .into_iter()
        .enumerate()
        {
            column.set_block_state(u32::try_from(index).unwrap(), 20, 0, state);
        }

        let packet = build_chunk_packet(&column, IVec2::new(3, -4)).unwrap();
        assert_eq!(packet.chunk.sections.len(), SECTION_COUNT);
        assert_eq!(packet.chunk.sections[0].non_empty_block_count, 4096);
        assert_eq!(
            packet.chunk.sections[0].block_states.get(0),
            i32::try_from(block_state::state_id("minecraft:stone", &[]).unwrap()).unwrap()
        );

        // The four blocks land in section 1, one of them water.
        assert_eq!(packet.chunk.sections[1].non_empty_block_count, 4);
        assert_eq!(packet.chunk.sections[1].fluid_count, 1);

        // Section 2 is untouched sky, so it stays air and the client is free
        // to skip drawing it.
        assert_eq!(packet.chunk.sections[2].non_empty_block_count, 0);

        let mut writer = Writer::new();
        packet.encode(&mut writer).unwrap();
        let bytes = writer.into_vec();

        let (states, biomes) = container_kinds();
        let mut reader = Reader::new(&bytes);
        let round_tripped = hyperion_minecraft_proto::world::LevelChunkWithLight::decode(
            SECTION_COUNT,
            states,
            biomes,
            &mut reader,
        )
        .unwrap();
        reader.finish().unwrap();
        assert_eq!(round_tripped, packet);
    }

    /// A column the anvil parse produced at the wrong height is refused, not
    /// truncated: the client reads exactly as many sections as its dimension
    /// says and would run off the end of the blob.
    #[test]
    fn a_column_of_the_wrong_height_is_refused() {
        let short = ColumnData::new(16 * 4);
        let error = build_chunk_packet(&short, IVec2::ZERO).unwrap_err();
        assert!(
            error.to_string().contains("has 4 sections"),
            "unexpected error: {error}"
        );
    }
}
