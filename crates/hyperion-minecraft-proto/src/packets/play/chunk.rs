//! Terrain: the chunks a client renders, and the block changes inside them.
//!
//! Five packets carry terrain, and only two of them need writing by hand.
//! `forget_level_chunk`, `chunk_batch_start`, `chunk_batch_finished` and
//! `block_update` are plain field sequences, so the generator produced them
//! into [`super::clientbound`] and they should be used from there.
//!
//! What is left is the two whose codecs branch on a runtime value:
//!
//! * `level_chunk_with_light` writes a length-prefixed blob whose length only
//!   the sections themselves know. It lives in [`crate::world::chunk`] because
//!   the paletted containers it is built from are world data rather than
//!   packet data, and is re-exported here so that a caller reaching for a
//!   terrain packet finds all of them in one place.
//! * [`SectionBlocksUpdate`] packs a state id and a position into one
//!   `VarLong` per change, which is a loop the generator will not write.

pub use crate::world::chunk::{
    BlockEntity, ChunkData, ChunkSection, Heightmap, HeightmapKind, LIGHT_LAYER_LEN,
    LevelChunkWithLight, LightData, MAX_SECTION_BLOB, light_mask,
};
use crate::{Decode, Encode, Error, Reader, Result, Writer};

/// The position of one 16-cubed section, packed into one `long`.
///
/// `SectionPos.STREAM_CODEC` is `ByteBufCodecs.LONG` over `SectionPos.asLong`,
/// which gives x the top 22 bits, z the next 22 and y the low 20. The widths
/// differ from [`crate::BlockPos`]'s because a section coordinate is a block
/// coordinate shifted right by four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SectionPos {
    /// Section x, which is block x divided by 16.
    pub x: i32,
    /// Section y, which is block y divided by 16.
    pub y: i32,
    /// Section z, which is block z divided by 16.
    pub z: i32,
}

/// `SectionPos.PACKED_X_LENGTH`, and the same for z.
const HORIZONTAL_BITS: u32 = 22;
/// `SectionPos.PACKED_Y_LENGTH`.
const Y_BITS: u32 = 64 - 2 * HORIZONTAL_BITS;
/// `SectionPos.X_OFFSET`.
const X_SHIFT: u32 = Y_BITS + HORIZONTAL_BITS;
/// `SectionPos.Z_OFFSET`.
const Z_SHIFT: u32 = Y_BITS;

impl SectionPos {
    /// A section at the given section coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The packed form written to the wire (`SectionPos.asLong`).
    #[must_use]
    pub const fn to_bits(self) -> i64 {
        let x = (self.x as i64 & mask(HORIZONTAL_BITS)) << X_SHIFT;
        let z = (self.z as i64 & mask(HORIZONTAL_BITS)) << Z_SHIFT;
        let y = self.y as i64 & mask(Y_BITS);
        x | z | y
    }

    /// Unpack the wire form, sign-extending each field from its own width.
    #[must_use]
    pub const fn from_bits(bits: i64) -> Self {
        Self {
            x: sign_extend(bits >> X_SHIFT, HORIZONTAL_BITS),
            y: sign_extend(bits, Y_BITS),
            z: sign_extend(bits >> Z_SHIFT, HORIZONTAL_BITS),
        }
    }
}

const fn mask(bits: u32) -> i64 {
    (1i64 << bits) - 1
}

/// Sign-extend the low `bits` of `value` as a two's-complement integer.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the shifts leave at most `bits` significant bits, and `bits` is under 32"
)]
const fn sign_extend(value: i64, bits: u32) -> i32 {
    (value << (64 - bits) >> (64 - bits)) as i32
}

impl Encode for SectionPos {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.to_bits());
        Ok(())
    }
}

impl Decode<'_> for SectionPos {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        reader.i64().map(Self::from_bits)
    }
}

/// One changed block within a section: where it is, and what it became.
///
/// Kept apart from its packed form because the packing is lossy in the
/// direction that matters: a caller that got the shift wrong would produce a
/// `VarLong` that still decodes, into a different block somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockChange {
    /// Section-relative x, 0 through 15.
    pub x: u8,
    /// Section-relative y, 0 through 15.
    pub y: u8,
    /// Section-relative z, 0 through 15.
    pub z: u8,
    /// The block state id the position now holds.
    pub state: u32,
}

/// `ClientboundSectionBlocksUpdatePacket.POS_IN_SECTION_BITS`.
const POS_IN_SECTION_BITS: u32 = 12;

impl BlockChange {
    /// The `VarLong` the packet carries: state id above the position.
    ///
    /// `write` is `Block.getId(state) << 12 | positions[i]`, where the
    /// position is `SectionPos`'s `x << 8 | z << 4 | y` packing.
    #[must_use]
    pub const fn to_bits(self) -> i64 {
        let position = ((self.x as i64) << 8) | ((self.z as i64) << 4) | self.y as i64;
        ((self.state as i64) << POS_IN_SECTION_BITS) | position
    }

    /// Unpack one `VarLong` from the packet.
    ///
    /// # Errors
    /// Returns [`Error::NegativeLength`] when the state id does not fit in a
    /// `u32`, which for a value the server wrote means the stream is
    /// desynchronised rather than that the block is unusual.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "each field is masked to its own width before the cast"
    )]
    pub const fn from_bits(bits: i64) -> Result<Self> {
        let position = bits & mask(POS_IN_SECTION_BITS);
        let state = bits >> POS_IN_SECTION_BITS;
        if state < 0 || state > u32::MAX as i64 {
            return Err(Error::NegativeLength(-1));
        }
        Ok(Self {
            x: ((position >> 8) & 0xF) as u8,
            y: (position & 0xF) as u8,
            z: ((position >> 4) & 0xF) as u8,
            state: state as u32,
        })
    }
}

/// `minecraft:section_blocks_update`, clientbound
/// (`ClientboundSectionBlocksUpdatePacket`).
///
/// Layout from
/// `net.minecraft.network.protocol.game.ClientboundSectionBlocksUpdatePacket#STREAM_CODEC`.
/// One packet per section, so a change spanning two sections is two packets.
/// The server sends this rather than a run of `block_update` once a tick's
/// changes in one section reach 64 (`ChunkHolder.broadcastChanges`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionBlocksUpdate {
    /// Which section the changes are relative to.
    pub section: SectionPos,
    /// The changed blocks, in any order.
    pub changes: Vec<BlockChange>,
}

impl Encode for SectionBlocksUpdate {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.section.encode(writer)?;
        writer.var_int(i32::try_from(self.changes.len()).map_err(|_| Error::NegativeLength(-1))?);
        for change in &self.changes {
            writer.var_long(change.to_bits());
        }
        Ok(())
    }
}

impl Decode<'_> for SectionBlocksUpdate {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let section = SectionPos::decode(reader)?;
        let count = reader.var_int()?;
        let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
        // A `VarLong` is at least one byte, so a count the frame cannot supply
        // is refused before anything is reserved.
        if count > reader.remaining_len() {
            return Err(Error::UnexpectedEof {
                needed: count,
                available: reader.remaining_len(),
            });
        }
        let mut changes = Vec::with_capacity(count);
        for _ in 0..count {
            changes.push(BlockChange::from_bits(reader.var_long()?)?);
        }
        Ok(Self { section, changes })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BlockChange, ChunkData, ChunkSection, Heightmap, HeightmapKind, LevelChunkWithLight,
        LightData, SectionBlocksUpdate, SectionPos,
    };
    use crate::{
        Decode, Encode, Reader, Writer,
        world::palette::{ContainerKind, PalettedContainer},
    };

    fn encoded(value: &impl Encode) -> Vec<u8> {
        let mut writer = Writer::new();
        value.encode(&mut writer).unwrap();
        writer.into_vec()
    }

    /// `SectionPos.asLong(1, -1, 2)`, computed from the field widths in
    /// `SectionPos`.
    #[test]
    fn a_section_pos_packs_x_high_and_y_low() {
        let packed = SectionPos::new(1, -1, 2).to_bits();
        assert_eq!(packed, (1i64 << 42) | (2i64 << 20) | 0xF_FFFF);
        assert_eq!(SectionPos::from_bits(packed), SectionPos::new(1, -1, 2));
    }

    /// The one full packet this server sends per chunk, pinned byte for byte.
    ///
    /// A single-section chunk of stone with one heightmap and no light. Every
    /// field is short enough to read off the hex, which is the point: nothing
    /// in this format is self-describing, so a change in any length prefix has
    /// to be visible here rather than in a client that renders nothing.
    #[test]
    fn a_one_section_chunk_pins_its_bytes() {
        let states = ContainerKind::block_states(32366);
        let biomes = ContainerKind::biomes(66);

        let packet = LevelChunkWithLight {
            x: 1,
            z: -2,
            chunk: ChunkData {
                heightmaps: vec![Heightmap {
                    kind: HeightmapKind::MotionBlocking,
                    data: vec![0x0102_0304_0506_0708],
                }],
                sections: vec![ChunkSection {
                    non_empty_block_count: 4096,
                    fluid_count: 0,
                    block_states: PalettedContainer::single(states, 1),
                    biomes: PalettedContainer::single(biomes, 0),
                }],
                block_entities: Vec::new(),
            },
            light: LightData::default(),
        };

        let bytes = encoded(&packet);
        assert_eq!(bytes, [
            // x = 1, z = -2, each a plain big-endian int
            0x00, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFE,
            // one heightmap: kind 4 (MOTION_BLOCKING), one long of data
            0x01, 0x04, 0x01, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            // the section blob: eight bytes, then the section itself
            0x08, // blob length
            0x10, 0x00, // 4096 non-empty blocks
            0x00, 0x00, // no fluid
            0x00, 0x01, // block states: zero bits, single value 1
            0x00, 0x00, // biomes: zero bits, single value 0
            // no block entities
            0x00, // four empty light masks and two empty update lists
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);

        let mut reader = Reader::new(&bytes);
        let round_tripped = LevelChunkWithLight::decode(1, states, biomes, &mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(round_tripped, packet);
    }

    /// The packed change: state id above the twelve position bits.
    #[test]
    fn a_block_change_packs_the_state_above_the_position() {
        let change = BlockChange {
            x: 1,
            y: 2,
            z: 3,
            state: 5,
        };
        // x << 8 | z << 4 | y is 0x132, and the state sits twelve bits up.
        assert_eq!(change.to_bits(), (5 << 12) | 0x132);
        assert_eq!(BlockChange::from_bits(change.to_bits()).unwrap(), change);
    }

    #[test]
    fn section_blocks_update_round_trips() {
        let packet = SectionBlocksUpdate {
            section: SectionPos::new(-3, 4, 5),
            changes: vec![
                BlockChange {
                    x: 0,
                    y: 0,
                    z: 0,
                    state: 0,
                },
                BlockChange {
                    x: 15,
                    y: 15,
                    z: 15,
                    state: 32_365,
                },
            ],
        };

        let bytes = encoded(&packet);
        let mut reader = Reader::new(&bytes);
        assert_eq!(SectionBlocksUpdate::decode(&mut reader).unwrap(), packet);
        reader.finish().unwrap();
    }
}
