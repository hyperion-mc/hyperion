//! `level_chunk_with_light`, the packet that makes a world visible.
//!
//! Transcribed from `ClientboundLevelChunkWithLightPacket`,
//! `ClientboundLevelChunkPacketData`, `ClientboundLightUpdatePacketData` and
//! `LevelChunkSection`.
//!
//! # Three places 26.2 differs from what is usually written down
//!
//! * **Heightmaps are not NBT.** `ClientboundLevelChunkPacketData` sends
//!   `ByteBufCodecs.map(.., Heightmap.Types.STREAM_CODEC, ByteBufCodecs.LONG_ARRAY)`:
//!   a count, then a `VarInt` type id and a length-prefixed `long[]` per
//!   entry. The NBT compound belongs to an older protocol.
//! * **A section carries two shorts, not one.** `LevelChunkSection.write`
//!   writes `nonEmptyBlockCount` *and* `fluidCount`, which is why
//!   `getSerializedSize` starts at 4.
//! * **A paletted container's storage has no length prefix.** See
//!   [`super::palette`].

use super::palette::{ContainerKind, PalettedContainer};
use crate::{Decode, Encode, Error, Reader, Result, Writer, nbt::Tag};

/// Bytes in one light array (`DataLayer.SIZE`): 4096 nibbles.
pub const LIGHT_LAYER_LEN: usize = 2048;

/// The cap `ClientboundLevelChunkPacketData` puts on the section blob.
pub const MAX_SECTION_BLOB: usize = 0x0020_0000;

/// A heightmap kind (`Heightmap.Types`).
///
/// The discriminants are `Heightmap.Types.id`, which
/// `ByteBufCodecs.idMapper` writes as a `VarInt`. Only the three marked
/// `Usage.CLIENT` are sent; the others are worldgen or server-side scratch,
/// and `sendToClient` filters them out before the map is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum HeightmapKind {
    /// Worldgen-only: highest non-air block.
    WorldSurfaceWg = 0,
    /// Highest non-air block. Sent to the client.
    WorldSurface = 1,
    /// Worldgen-only: highest motion-blocking block.
    OceanFloorWg = 2,
    /// Highest motion-blocking block. Server-side only.
    OceanFloor = 3,
    /// Highest motion-blocking or fluid block. Sent to the client.
    MotionBlocking = 4,
    /// As [`MotionBlocking`](Self::MotionBlocking) but ignoring leaves. Sent
    /// to the client.
    MotionBlockingNoLeaves = 5,
}

impl HeightmapKind {
    /// The three `Usage.CLIENT` kinds, which are the ones this packet carries.
    pub const CLIENT: [Self; 3] = [
        Self::WorldSurface,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];

    /// True for the kinds `Heightmap.Types.sendToClient` accepts.
    #[must_use]
    pub const fn send_to_client(self) -> bool {
        matches!(
            self,
            Self::WorldSurface | Self::MotionBlocking | Self::MotionBlockingNoLeaves
        )
    }

    /// The kind with this network id.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] for an id no kind carries. The server
    /// itself clamps instead (`ByIdMap.OutOfBoundsStrategy.ZERO`), but
    /// clamping here would silently turn a corrupt packet into a valid one.
    pub const fn from_id(id: i32) -> Result<Self> {
        match id {
            0 => Ok(Self::WorldSurfaceWg),
            1 => Ok(Self::WorldSurface),
            2 => Ok(Self::OceanFloorWg),
            3 => Ok(Self::OceanFloor),
            4 => Ok(Self::MotionBlocking),
            5 => Ok(Self::MotionBlockingNoLeaves),
            _ => Err(Error::InvalidEnum {
                name: "Heightmap.Types",
                value: id,
            }),
        }
    }
}

/// One heightmap: a kind and the packed column heights.
///
/// The `long[]` is a `SimpleBitStorage` of 256 columns at
/// `ceillog2(worldHeight + 1)` bits each, which for a 384-block overworld is
/// nine bits and 37 longs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heightmap {
    /// Which heightmap this is.
    pub kind: HeightmapKind,
    /// `Heightmap.getRawData()`, the bit storage backing the 256 columns.
    pub data: Vec<i64>,
}

/// One block entity in a chunk (`ClientboundLevelChunkPacketData.BlockEntityInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockEntity<'a> {
    /// Section-relative x in the high nibble and z in the low one.
    pub packed_xz: u8,
    /// Absolute y.
    pub y: i16,
    /// Network id in `minecraft:block_entity_type`.
    pub kind: i32,
    /// The block entity's update tag, or `None` when it is empty.
    ///
    /// `BlockEntityInfo.create` substitutes `null` for an empty tag, and
    /// `writeNbt` writes that as a bare `TAG_End`.
    pub tag: Option<Tag<'a>>,
}

impl Encode for BlockEntity<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(self.packed_xz);
        writer.i16(self.y);
        writer.var_int(self.kind);
        match &self.tag {
            Some(tag) => tag.encode(writer)?,
            None => writer.u8(crate::nbt::TAG_END),
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for BlockEntity<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            packed_xz: reader.u8()?,
            y: reader.i16()?,
            kind: reader.var_int()?,
            tag: crate::nbt::decode_optional(reader)?,
        })
    }
}

/// One 16-cubed slice of a chunk (`LevelChunkSection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSection {
    /// Blocks that are not air. A client uses it to skip empty sections.
    pub non_empty_block_count: i16,
    /// Blocks carrying a fluid.
    pub fluid_count: i16,
    /// Block states, 4096 entries.
    pub block_states: PalettedContainer,
    /// Biomes, 64 entries.
    pub biomes: PalettedContainer,
}

impl ChunkSection {
    /// Bytes this section occupies (`LevelChunkSection.getSerializedSize`).
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        4 + self.block_states.encoded_len() + self.biomes.encoded_len()
    }

    /// Read one section.
    ///
    /// The kinds have to be supplied because they are not on the wire: a
    /// reader has to already know the entry count and the palette tables, and
    /// the two containers disagree about both.
    ///
    /// # Errors
    /// As [`PalettedContainer::decode`].
    pub fn decode(
        block_states: ContainerKind,
        biomes: ContainerKind,
        reader: &mut Reader<'_>,
    ) -> Result<Self> {
        Ok(Self {
            non_empty_block_count: reader.i16()?,
            fluid_count: reader.i16()?,
            block_states: PalettedContainer::decode(block_states, reader)?,
            biomes: PalettedContainer::decode(biomes, reader)?,
        })
    }
}

impl Encode for ChunkSection {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i16(self.non_empty_block_count);
        writer.i16(self.fluid_count);
        self.block_states.encode(writer)?;
        self.biomes.encode(writer)?;
        Ok(())
    }
}

/// The chunk half of the packet (`ClientboundLevelChunkPacketData`).
///
/// `sections` is kept as a decoded list rather than the opaque blob the packet
/// carries. The blob is length-prefixed and its length is not derivable from
/// anything else, so encoding measures the sections first.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkData<'a> {
    /// Heightmaps to send. Only [`HeightmapKind::CLIENT`] kinds belong here.
    pub heightmaps: Vec<Heightmap>,
    /// Sections from the world's minimum section upwards, with no gaps.
    pub sections: Vec<ChunkSection>,
    /// Block entities within the chunk.
    pub block_entities: Vec<BlockEntity<'a>>,
}

impl Encode for ChunkData<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer
            .var_int(i32::try_from(self.heightmaps.len()).map_err(|_| Error::NegativeLength(-1))?);
        for heightmap in &self.heightmaps {
            writer.var_int(heightmap.kind as i32);
            writer.var_int(
                i32::try_from(heightmap.data.len()).map_err(|_| Error::NegativeLength(-1))?,
            );
            for word in &heightmap.data {
                writer.i64(*word);
            }
        }

        // `extractChunkData` asserts the blob is exactly as long as
        // `calculateChunkSize` predicted. Encoding into a scratch buffer makes
        // the same guarantee structurally, so the two cannot drift.
        let mut blob = Writer::new();
        for section in &self.sections {
            section.encode(&mut blob)?;
        }
        let blob = blob.into_vec();
        if blob.len() > MAX_SECTION_BLOB {
            return Err(Error::NegativeLength(
                i32::try_from(blob.len()).unwrap_or(-1),
            ));
        }
        writer.byte_array(&blob)?;

        writer.var_int(
            i32::try_from(self.block_entities.len()).map_err(|_| Error::NegativeLength(-1))?,
        );
        for entity in &self.block_entities {
            entity.encode(writer)?;
        }
        Ok(())
    }
}

impl<'a> ChunkData<'a> {
    /// Read the chunk half, splitting the section blob into `section_count`
    /// sections.
    ///
    /// The count is not on the wire. A client knows it from the dimension's
    /// height, which arrived in `registry_data`; this decoder is told.
    ///
    /// # Errors
    /// Returns [`Error::TrailingBytes`] when the sections do not exactly fill
    /// the blob, which is the check `extractChunkData` makes on the way out.
    pub fn decode(
        section_count: usize,
        block_states: ContainerKind,
        biomes: ContainerKind,
        reader: &mut Reader<'a>,
    ) -> Result<Self> {
        let count = reader.var_int()?;
        let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
        // Each entry is at least a type id and a zero-length array, so two
        // bytes; anything the frame cannot supply is refused before reserving.
        if count.saturating_mul(2) > reader.remaining_len() {
            return Err(Error::UnexpectedEof {
                needed: count * 2,
                available: reader.remaining_len(),
            });
        }
        let mut heightmaps = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = HeightmapKind::from_id(reader.var_int()?)?;
            let words = reader.var_int()?;
            let words = usize::try_from(words).map_err(|_| Error::NegativeLength(words))?;
            if words.saturating_mul(8) > reader.remaining_len() {
                return Err(Error::UnexpectedEof {
                    needed: words * 8,
                    available: reader.remaining_len(),
                });
            }
            let mut data = Vec::with_capacity(words);
            for _ in 0..words {
                data.push(reader.i64()?);
            }
            heightmaps.push(Heightmap { kind, data });
        }

        let blob = reader.byte_array()?;
        let mut blob_reader = Reader::new(blob);
        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            sections.push(ChunkSection::decode(
                block_states,
                biomes,
                &mut blob_reader,
            )?);
        }
        blob_reader.finish()?;

        let count = reader.var_int()?;
        let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
        // Four bytes minimum: packed xz, two of y, a type id and a TAG_End.
        if count.saturating_mul(5) > reader.remaining_len() {
            return Err(Error::UnexpectedEof {
                needed: count * 5,
                available: reader.remaining_len(),
            });
        }
        let mut block_entities = Vec::with_capacity(count);
        for _ in 0..count {
            block_entities.push(BlockEntity::decode(reader)?);
        }

        Ok(Self {
            heightmaps,
            sections,
            block_entities,
        })
    }
}

/// The light half of the packet (`ClientboundLightUpdatePacketData`).
///
/// The masks are indexed by *light* section, of which there are two more than
/// there are chunk sections: `LevelLightEngine` keeps one below the world and
/// one above so that light can spill in from outside.
///
/// A section appears in exactly one of each pair. Being in `sky` or `block`
/// means an array follows, in order; being in `empty_sky` or `empty_block`
/// means the section is uniformly dark and no array is sent. A section in
/// neither has no data at all, which is how a server says "unchanged".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LightData {
    /// Sections with a sky-light array, low bit first.
    pub sky_mask: Vec<i64>,
    /// Sections with a block-light array.
    pub block_mask: Vec<i64>,
    /// Sections whose sky light is uniformly zero.
    pub empty_sky_mask: Vec<i64>,
    /// Sections whose block light is uniformly zero.
    pub empty_block_mask: Vec<i64>,
    /// One 2048-byte array per bit set in `sky_mask`, in ascending order.
    pub sky_updates: Vec<Vec<u8>>,
    /// One 2048-byte array per bit set in `block_mask`.
    pub block_updates: Vec<Vec<u8>>,
}

/// Pack section indices into the `long[]` form `BitSet.toLongArray` produces.
///
/// Trailing zero words are dropped, which is why an all-clear mask is written
/// as a zero-length array rather than as words of zeroes.
#[must_use]
pub fn light_mask(sections: &[usize]) -> Vec<i64> {
    let Some(highest) = sections.iter().copied().max() else {
        return Vec::new();
    };
    let mut words = vec![0i64; highest / 64 + 1];
    for section in sections {
        words[section / 64] |= 1i64 << (section % 64);
    }
    words
}

fn write_long_array(writer: &mut Writer, words: &[i64]) -> Result<()> {
    writer.var_int(i32::try_from(words.len()).map_err(|_| Error::NegativeLength(-1))?);
    for word in words {
        writer.i64(*word);
    }
    Ok(())
}

fn read_long_array(reader: &mut Reader<'_>) -> Result<Vec<i64>> {
    let count = reader.var_int()?;
    let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
    // `FriendlyByteBuf.readLongArray` makes exactly this check before
    // allocating, and for the same reason.
    if count.saturating_mul(8) > reader.remaining_len() {
        return Err(Error::UnexpectedEof {
            needed: count * 8,
            available: reader.remaining_len(),
        });
    }
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        words.push(reader.i64()?);
    }
    Ok(words)
}

fn write_layers(writer: &mut Writer, layers: &[Vec<u8>]) -> Result<()> {
    writer.var_int(i32::try_from(layers.len()).map_err(|_| Error::NegativeLength(-1))?);
    for layer in layers {
        if layer.len() != LIGHT_LAYER_LEN {
            return Err(Error::NegativeLength(
                i32::try_from(layer.len()).unwrap_or(-1),
            ));
        }
        writer.byte_array(layer)?;
    }
    Ok(())
}

fn read_layers(reader: &mut Reader<'_>) -> Result<Vec<Vec<u8>>> {
    let count = reader.var_int()?;
    let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
    if count.saturating_mul(LIGHT_LAYER_LEN) > reader.remaining_len() {
        return Err(Error::UnexpectedEof {
            needed: count * LIGHT_LAYER_LEN,
            available: reader.remaining_len(),
        });
    }
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        let layer = reader.byte_array()?;
        if layer.len() != LIGHT_LAYER_LEN {
            return Err(Error::NegativeLength(
                i32::try_from(layer.len()).unwrap_or(-1),
            ));
        }
        layers.push(layer.to_vec());
    }
    Ok(layers)
}

impl Encode for LightData {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        write_long_array(writer, &self.sky_mask)?;
        write_long_array(writer, &self.block_mask)?;
        write_long_array(writer, &self.empty_sky_mask)?;
        write_long_array(writer, &self.empty_block_mask)?;
        write_layers(writer, &self.sky_updates)?;
        write_layers(writer, &self.block_updates)?;
        Ok(())
    }
}

impl Decode<'_> for LightData {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            sky_mask: read_long_array(reader)?,
            block_mask: read_long_array(reader)?,
            empty_sky_mask: read_long_array(reader)?,
            empty_block_mask: read_long_array(reader)?,
            sky_updates: read_layers(reader)?,
            block_updates: read_layers(reader)?,
        })
    }
}

/// `minecraft:level_chunk_with_light`, clientbound
/// (`ClientboundLevelChunkWithLightPacket`).
///
/// The single biggest packet a server sends, and the one without which a
/// client stands in an empty world.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelChunkWithLight<'a> {
    /// Chunk x, in chunks. A plain big-endian int, not a `VarInt`.
    pub x: i32,
    /// Chunk z, in chunks.
    pub z: i32,
    /// Blocks, biomes, heightmaps and block entities.
    pub chunk: ChunkData<'a>,
    /// Sky and block light.
    pub light: LightData,
}

impl Encode for LevelChunkWithLight<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i32(self.x);
        writer.i32(self.z);
        self.chunk.encode(writer)?;
        self.light.encode(writer)?;
        Ok(())
    }
}

impl<'a> LevelChunkWithLight<'a> {
    /// Read the packet, splitting the blob into `section_count` sections.
    ///
    /// # Errors
    /// As [`ChunkData::decode`].
    pub fn decode(
        section_count: usize,
        block_states: ContainerKind,
        biomes: ContainerKind,
        reader: &mut Reader<'a>,
    ) -> Result<Self> {
        Ok(Self {
            x: reader.i32()?,
            z: reader.i32()?,
            chunk: ChunkData::decode(section_count, block_states, biomes, reader)?,
            light: LightData::decode(reader)?,
        })
    }
}
