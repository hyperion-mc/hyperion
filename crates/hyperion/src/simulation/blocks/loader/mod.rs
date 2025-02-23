use std::{borrow::Cow, cell::RefCell, io::Write, sync::Arc};

use anyhow::{Context, bail};
use bytes::BytesMut;
use derive_more::Constructor;
use glam::{I16Vec2, IVec2};
use hyperion_nerd_font::NERD_ROCKET;
use itertools::Itertools;
use libdeflater::{CompressionLvl, Compressor};
use parse::ColumnData;
use rustc_hash::FxHashSet;
use tracing::{debug, warn};
use valence_generated::block::BlockState;
use valence_nbt::{List, compound};
use valence_protocol::{ChunkPos, CompressionThreshold, FixedArray, packets::play};
use valence_registry::RegistryIdx;
use valence_server::layer::chunk::{BiomeContainer, Chunk, bit_width};

pub mod parse;

use super::{chunk::Column, shared::WorldShared};
use crate::{
    CHUNK_HEIGHT_SPAN, Scratch,
    net::encoder::PacketEncoder,
    runtime::AsyncRuntime,
    simulation::{blocks::loader::parse::section::Section, util::heightmap},
    storage::BitStorage,
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

fn encode_chunk_packet(
    chunk: &ColumnData,
    location: IVec2,
    state: &mut TasksState,
) -> anyhow::Result<Option<BytesMut>> {
    let encoder = PacketEncoder::new(CompressionThreshold::from(6));

    let section_count = CHUNK_HEIGHT_SPAN as usize / 16_usize;
    let dimension_height = CHUNK_HEIGHT_SPAN;

    let map = heightmap(dimension_height, dimension_height - 3);
    let map = map.into_iter().map(i64::try_from).try_collect()?;

    // convert section_count + 2 0b1s into `u64` array
    // todo: this is jank let's do the non jank way so we can get smaller packet sizes
    let mut sky_light_mask = BitStorage::new(1, section_count + 2, None)?;
    let mut block_light_mask = BitStorage::new(1, section_count + 2, None)?;

    // 2048 bytes per section -> long count = 2048 / 8 = 256
    // let empty_light = FixedArray([0x00_u8; 2048]);

    let mut sky_light_arrays = vec![];
    let mut block_light_arrays = vec![];

    let mut section_bytes = Vec::new();

    sky_light_mask.set(0, 0);

    block_light_mask.set(0, 0);

    for (i, section) in chunk.sections.iter().enumerate() {
        use valence_protocol::Encode;
        let non_air_blocks: u16 = 42;
        non_air_blocks.encode(&mut section_bytes).unwrap();

        // todo: how do sky light and block light work differently?

        if let Some(sky_light) = section.sky_light {
            let sky_light = FixedArray(sky_light);
            sky_light_arrays.push(sky_light);
        } else {
            // if there is no sky light, let's assume it is full bright for now
            sky_light_arrays.push(FixedArray([0xff; 2048]));
        }
        sky_light_mask.set(i + 1, 1);

        if let Some(block_light) = section.block_light {
            let block_light = FixedArray(block_light);
            block_light_arrays.push(block_light);
            block_light_mask.set(i + 1, 1);
        }

        write_block_states(&section.block_states, &mut section_bytes).unwrap();
        write_biomes(&section.biomes, &mut section_bytes).unwrap();
    }

    // todo: is this right?
    sky_light_mask.set(section_count + 1, 1);
    sky_light_arrays.push(FixedArray([0xff; 2048]));

    block_light_mask.set(section_count + 1, 0);

    // todo: Maybe we want the top one to actually be all Fs because I think this is just an edge case for how things are rendered.
    // sky_light_arrays.push(empty_light);
    // block_light_arrays.push(empty_light);

    // debug_assert_eq!(sky_light_arrays.len(), section_count + 2);
    // debug_assert_eq!(block_light_arrays.len(), section_count + 2);

    let sky_light_data = sky_light_mask.into_data();
    let block_light_data = block_light_mask.into_data();

    let pkt = play::ChunkDataS2c {
        pos: ChunkPos::new(location.x, location.y),

        // todo: I think this is for rain and snow???
        heightmaps: Cow::Owned(compound! {
            "MOTION_BLOCKING" => List::Long(map),
        }),
        blocks_and_biomes: &section_bytes,
        block_entities: Cow::Borrowed(&[]),

        sky_light_mask: Cow::Borrowed(&sky_light_data),
        block_light_mask: Cow::Borrowed(&block_light_data),

        empty_sky_light_mask: Cow::Borrowed(&[]),
        empty_block_light_mask: Cow::Borrowed(&[]),

        sky_light_arrays: Cow::Owned(sky_light_arrays),
        block_light_arrays: Cow::Owned(block_light_arrays),
    };

    let buf = &mut state.bytes;
    let scratch = &mut state.scratch;
    let compressor = &mut state.compressor;

    let result = encoder.append_packet(&pkt, buf, scratch, compressor)?;

    Ok(Some(result))
}

fn write_block_states(
    states: &hyperion_palette::PalettedContainer,
    writer: &mut impl Write,
) -> anyhow::Result<()> {
    states.encode_mc_format(
        writer,
        Into::into,
        4,
        8,
        bit_width(BlockState::max_raw().into()),
    )?;
    Ok(())
}

fn write_biomes(biomes: &BiomeContainer, writer: &mut impl Write) -> anyhow::Result<()> {
    biomes.encode_mc_format(
        writer,
        |b| b.to_index() as u64,
        0,
        3,
        6, // bit_width(info.biome_registry_len - 1),
    )?;
    Ok(())
}
