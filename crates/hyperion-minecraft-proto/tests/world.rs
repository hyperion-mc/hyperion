//! Chunk data, against bytes from the server's own codecs.
//!
//! The fixtures come from driving `PalettedContainer.write`,
//! `ByteBufCodecs.map` over `Heightmap.Types` and `FriendlyByteBuf.writeBitSet`
//! in the pinned server jar. None of this format has a checksum or a redundant
//! length, so an off-by-one in the packing shows up as terrain that is subtly
//! wrong rather than as an error, which is why it is pinned byte for byte.

mod vanilla_fixtures;

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Writer,
    registry_data,
    world::{
        ChunkData, ChunkSection, ContainerKind, Heightmap, HeightmapKind, LevelChunkWithLight,
        LightData, PalettedContainer, light_mask, storage_len,
    },
};

use vanilla_fixtures as vanilla;

/// `Block.BLOCK_STATE_REGISTRY.size()` only sets the width of the global
/// palette, which none of these containers reaches. The number is not in the
/// generated tables -- those carry block *names* -- so it is named here rather
/// than pretended to be derived.
const BLOCK_STATE_COUNT: usize = 30_000;

fn block_states() -> ContainerKind {
    ContainerKind::block_states(BLOCK_STATE_COUNT)
}

fn biomes() -> ContainerKind {
    ContainerKind::biomes(registry_data::WORLDGEN_BIOME.len())
}

fn encoded(value: &impl Encode) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

// --- paletted containers ---------------------------------------------------

#[test]
fn a_uniform_section_is_three_bytes() {
    let air = vanilla::number("block_state_id.air");
    let container = PalettedContainer::single(block_states(), air);
    let bytes = encoded(&container);

    assert_eq!(vanilla::hex(&bytes), vanilla::get("palette.single_air"));
    assert_eq!(bytes.len(), container.encoded_len());

    let stone = vanilla::number("block_state_id.stone");
    assert_eq!(
        vanilla::hex(&encoded(&PalettedContainer::single(block_states(), stone))),
        vanilla::get("palette.single_stone")
    );
}

/// Two distinct states in a section, which `Strategy.createForBlockStates`
/// maps to a linear palette padded to four bits. The bit width in the header
/// is therefore 4 and not 1, which is the step most transcriptions get wrong.
#[test]
fn a_two_entry_palette_is_written_at_four_bits() {
    let air = vanilla::number("block_state_id.air");
    let stone = vanilla::number("block_state_id.stone");
    let kind = block_states();

    let mut values = vec![air; kind.entry_count()];
    values[kind.index(0, 0, 0)] = stone;

    let container = PalettedContainer::from_values(kind, air, &values).expect("build");
    assert_eq!(container.bits(), 4);
    assert_eq!(
        vanilla::hex(&encoded(&container)),
        vanilla::get("palette.linear_two")
    );
}

#[test]
fn a_three_entry_palette_packs_indices_across_the_section() {
    let air = vanilla::number("block_state_id.air");
    let stone = vanilla::number("block_state_id.stone");
    let dirt = vanilla::number("block_state_id.dirt");
    let kind = block_states();

    let mut values = vec![air; kind.entry_count()];
    values[kind.index(0, 0, 0)] = stone;
    values[kind.index(1, 0, 0)] = dirt;
    values[kind.index(15, 15, 15)] = dirt;

    let container = PalettedContainer::from_values(kind, air, &values).expect("build");
    assert_eq!(
        vanilla::hex(&encoded(&container)),
        vanilla::get("palette.linear_three")
    );
}

/// Biomes are a 4-cubed container with its own bit table, so the same palette
/// size produces a different header. One bit here where block states use four.
#[test]
fn biome_containers_use_their_own_bit_table() {
    let plains = vanilla::number("biome_id.plains");
    let desert = vanilla::number("biome_id.desert");
    let kind = biomes();

    assert_eq!(kind.entry_count(), 64);
    assert_eq!(
        vanilla::hex(&encoded(&PalettedContainer::single(kind, plains))),
        vanilla::get("palette.biome_single")
    );

    let mut values = vec![plains; kind.entry_count()];
    values[kind.index(0, 0, 0)] = desert;
    let container = PalettedContainer::from_values(kind, plains, &values).expect("build");
    assert_eq!(container.bits(), 1);
    assert_eq!(
        vanilla::hex(&encoded(&container)),
        vanilla::get("palette.biome_linear_two")
    );
}

/// The storage has no length prefix, so the reader has to derive its size.
/// Getting that wrong reads into the next container and produces no error at
/// all until the frame runs short, several sections later.
#[test]
fn the_storage_length_is_derived_and_not_read() {
    let kind = block_states();
    assert_eq!(storage_len(0, kind.entry_count()), 0);
    assert_eq!(storage_len(4, kind.entry_count()), 256);
    // 64 / 5 is 12 values per long, so 4096 values need 342 longs and the last
    // one is only two thirds used.
    assert_eq!(storage_len(5, kind.entry_count()), 342);
    assert_eq!(storage_len(1, biomes().entry_count()), 1);

    let bytes = vanilla::bytes("palette.linear_two");
    // One byte of width, one of palette size, two of palette, then the longs.
    assert_eq!(bytes.len(), 1 + 1 + 2 + 256 * 8);
}

#[test]
fn containers_round_trip() {
    let air = vanilla::number("block_state_id.air");
    let stone = vanilla::number("block_state_id.stone");
    let kind = block_states();

    let mut values = vec![air; kind.entry_count()];
    for (index, value) in values.iter_mut().enumerate() {
        if index.is_multiple_of(3) {
            *value = stone;
        }
    }

    let container = PalettedContainer::from_values(kind, air, &values).expect("build");
    let bytes = encoded(&container);
    let mut reader = Reader::new(&bytes);
    let decoded = PalettedContainer::decode(kind, &mut reader).expect("decode");
    reader.finish().expect("fully consumed");

    assert_eq!(decoded, container);
    for (index, value) in values.iter().enumerate() {
        assert_eq!(decoded.get(index), *value, "entry {index}");
    }
}

/// Past 256 distinct states the palette is dropped and the storage holds
/// registry ids directly, at the width the whole registry needs.
#[test]
fn a_dense_section_falls_back_to_the_global_palette() {
    let kind = block_states();
    let values: Vec<i32> = (0..kind.entry_count())
        .map(|index| i32::try_from(index % 400).unwrap())
        .collect();

    let container = PalettedContainer::from_values(kind, 0, &values).expect("build");
    assert!(kind.is_global(container.bits()), "expected a global palette");
    assert!(container.palette().is_empty(), "no palette is written");

    let bytes = encoded(&container);
    assert_eq!(bytes.len(), 1 + storage_len(container.bits(), 4096) * 8);

    let mut reader = Reader::new(&bytes);
    let decoded = PalettedContainer::decode(kind, &mut reader).expect("decode");
    reader.finish().expect("fully consumed");
    for (index, value) in values.iter().enumerate() {
        assert_eq!(decoded.get(index), *value, "entry {index}");
    }
}

// --- heightmaps ------------------------------------------------------------

/// In 26.2 the heightmaps are a `VarInt`-keyed map of `long[]`, not the NBT
/// compound older protocols sent.
#[test]
fn heightmaps_match_the_map_codec() {
    let mut writer = Writer::new();
    let empty: Vec<Heightmap> = Vec::new();
    ChunkData {
        heightmaps: empty,
        sections: Vec::new(),
        block_entities: Vec::new(),
    }
    .encode(&mut writer)
    .expect("encode");
    // The first byte is the map's own count; the rest is the empty section
    // blob and the empty block-entity list.
    assert_eq!(vanilla::hex(&writer.as_slice()[..1]), vanilla::get("heightmaps.empty"));

    let mut writer = Writer::new();
    ChunkData {
        heightmaps: vec![
            Heightmap {
                kind: HeightmapKind::WorldSurface,
                data: vec![1, 2],
            },
            Heightmap {
                kind: HeightmapKind::MotionBlocking,
                data: (0..37).map(|index| 0x0123_4567_89AB_CDEFi64 + index).collect(),
            },
        ],
        sections: Vec::new(),
        block_entities: Vec::new(),
    }
    .encode(&mut writer)
    .expect("encode");

    let expected = vanilla::bytes("heightmaps.two");
    assert_eq!(vanilla::hex(&writer.as_slice()[..expected.len()]), vanilla::hex(&expected));
}

#[test]
fn only_three_heightmap_kinds_go_to_the_client() {
    for (name, id) in [
        ("WORLD_SURFACE_WG", HeightmapKind::WorldSurfaceWg),
        ("WORLD_SURFACE", HeightmapKind::WorldSurface),
        ("OCEAN_FLOOR_WG", HeightmapKind::OceanFloorWg),
        ("OCEAN_FLOOR", HeightmapKind::OceanFloor),
        ("MOTION_BLOCKING", HeightmapKind::MotionBlocking),
        ("MOTION_BLOCKING_NO_LEAVES", HeightmapKind::MotionBlockingNoLeaves),
    ] {
        assert_eq!(id as i32, vanilla::number(&format!("heightmap_id.{name}")));
        assert_eq!(
            id.send_to_client().to_string(),
            vanilla::get(&format!("heightmap_client.{name}")),
            "{name}"
        );
    }
    assert_eq!(
        HeightmapKind::CLIENT.len(),
        3,
        "the CLIENT list and send_to_client must agree"
    );
}

// --- light -----------------------------------------------------------------

#[test]
fn light_masks_match_the_bitset_codec() {
    let mut writer = Writer::new();
    LightData {
        sky_mask: light_mask(&[0, 3, 25]),
        ..LightData::default()
    }
    .encode(&mut writer)
    .expect("encode");

    let expected = vanilla::bytes("bitset.0_3_25");
    assert_eq!(vanilla::hex(&writer.as_slice()[..expected.len()]), vanilla::hex(&expected));

    // `BitSet.toLongArray` drops trailing zero words, so an all-clear mask is
    // a zero-length array rather than a word of zeroes.
    assert!(light_mask(&[]).is_empty());
    let mut writer = Writer::new();
    LightData::default().encode(&mut writer).expect("encode");
    assert_eq!(
        vanilla::hex(&writer.as_slice()[..1]),
        vanilla::get("bitset.empty")
    );
}

#[test]
fn light_arrays_are_length_prefixed_and_exactly_2048_bytes() {
    let layer: Vec<u8> = (0..2048).map(|index| (index & 0xFF) as u8).collect();
    let mut writer = Writer::new();
    writer.var_int(1);
    writer.byte_array(&layer).expect("write");
    assert_eq!(
        vanilla::hex(writer.as_slice()),
        vanilla::get("light.one_layer")
    );

    let data = LightData {
        sky_mask: light_mask(&[0]),
        sky_updates: vec![layer.clone()],
        empty_block_mask: light_mask(&[1, 2]),
        ..LightData::default()
    };
    let bytes = encoded(&data);
    let mut reader = Reader::new(&bytes);
    let decoded = LightData::decode(&mut reader).expect("decode");
    reader.finish().expect("fully consumed");
    assert_eq!(decoded, data);

    // A layer that is not exactly 2048 bytes is a bug in the caller, not a
    // shorter array on the wire.
    let bad = LightData {
        sky_mask: light_mask(&[0]),
        sky_updates: vec![vec![0u8; 1024]],
        ..LightData::default()
    };
    let mut writer = Writer::new();
    assert!(bad.encode(&mut writer).is_err());
}

// --- the whole packet ------------------------------------------------------

#[test]
fn a_chunk_round_trips() {
    let air = vanilla::number("block_state_id.air");
    let stone = vanilla::number("block_state_id.stone");
    let plains = vanilla::number("biome_id.plains");
    let states = block_states();
    let biome_kind = biomes();

    // A 24-section overworld column: the bottom four solid, the rest air.
    let sections: Vec<ChunkSection> = (0..24)
        .map(|section| {
            let solid = section < 4;
            let block_states = if solid {
                PalettedContainer::single(states, stone)
            } else {
                PalettedContainer::single(states, air)
            };
            ChunkSection {
                non_empty_block_count: if solid { 4096 } else { 0 },
                fluid_count: 0,
                block_states,
                biomes: PalettedContainer::single(biome_kind, plains),
            }
        })
        .collect();

    let packet = LevelChunkWithLight {
        x: -3,
        z: 7,
        chunk: ChunkData {
            heightmaps: vec![Heightmap {
                kind: HeightmapKind::MotionBlocking,
                data: vec![0x0F0F_0F0F_0F0F_0F0F; 37],
            }],
            sections,
            block_entities: Vec::new(),
        },
        light: LightData {
            // 26 light sections for 24 chunk sections: one below the world and
            // one above, so light can spill in from outside.
            empty_sky_mask: light_mask(&(0..26).collect::<Vec<_>>()),
            empty_block_mask: light_mask(&(0..26).collect::<Vec<_>>()),
            ..LightData::default()
        },
    };

    let bytes = encoded(&packet);
    let mut reader = Reader::new(&bytes);
    let decoded =
        LevelChunkWithLight::decode(24, states, biome_kind, &mut reader).expect("decode");
    reader.finish().expect("fully consumed");
    assert_eq!(decoded, packet);

    // Every section is uniform, so the blob is 24 sections of two shorts and
    // two three-byte containers.
    assert_eq!(
        packet.chunk.sections.iter().map(ChunkSection::encoded_len).sum::<usize>(),
        24 * (4 + 2 + 2)
    );
}

/// The section blob is length-prefixed but the number of sections inside it is
/// not, so a reader told the wrong count runs off the end of the blob rather
/// than off the end of the frame. That has to be an error, not a short read.
#[test]
fn the_wrong_section_count_is_refused() {
    let states = block_states();
    let biome_kind = biomes();
    let packet = LevelChunkWithLight {
        x: 0,
        z: 0,
        chunk: ChunkData {
            heightmaps: Vec::new(),
            sections: vec![ChunkSection {
                non_empty_block_count: 0,
                fluid_count: 0,
                block_states: PalettedContainer::single(states, 0),
                biomes: PalettedContainer::single(biome_kind, 0),
            }],
            block_entities: Vec::new(),
        },
        light: LightData::default(),
    };
    let bytes = encoded(&packet);

    let mut reader = Reader::new(&bytes);
    assert!(LevelChunkWithLight::decode(2, states, biome_kind, &mut reader).is_err());

    let mut reader = Reader::new(&bytes);
    assert!(LevelChunkWithLight::decode(0, states, biome_kind, &mut reader).is_err());
}
