//! Paletted containers, the format both block states and biomes are sent in.
//!
//! A container is a palette plus a bit-packed array of indices into it. Which
//! palette a container uses is not on the wire: the reader picks it from the
//! bit width in the header, using the same table the writer did. That table
//! lives in `Strategy.createForBlockStates` and `Strategy.createForBiomes` and
//! is different for the two, which is why [`ContainerKind`] exists.
//!
//! # What changed in 26.x
//!
//! `PalettedContainer.Data.write` ends with `writeFixedSizeLongArray`, not
//! `writeLongArray`. There is **no `VarInt` count before the longs**; the
//! reader knows how many to expect because the bit width and the entry count
//! determine it. Wiki pages describing a length-prefixed array are describing
//! an older protocol.

use crate::{Encode, Error, Reader, Result, Writer};

/// Which of the two container shapes a value is being stored in.
///
/// The two differ in three ways, all of them consequences of
/// `Strategy`: how many entries a container holds, which palette each bit
/// width selects, and how wide the global palette is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerKind {
    /// `Strategy.bitsPerAxis`: 4 for block states, 2 for biomes.
    bits_per_axis: u32,
    /// `Strategy.globalPaletteBitsInMemory`, which is
    /// `Mth.ceillog2(registry.size())`.
    global_bits: u32,
}

impl ContainerKind {
    /// Block states: a 16-cubed container over the block state registry.
    ///
    /// `state_count` is `Block.BLOCK_STATE_REGISTRY.size()`, the number of
    /// distinct block *states* rather than blocks. It only affects the width
    /// of the global palette, which a container reaches at more than 256
    /// distinct states in one section.
    #[must_use]
    pub const fn block_states(state_count: usize) -> Self {
        Self {
            bits_per_axis: 4,
            global_bits: ceil_log2(state_count),
        }
    }

    /// Biomes: a 4-cubed container over the biome registry.
    ///
    /// `biome_count` is the size of the synchronised biome registry, which
    /// for vanilla is [`crate::registry_data::WORLDGEN_BIOME`]'s length.
    #[must_use]
    pub const fn biomes(biome_count: usize) -> Self {
        Self {
            bits_per_axis: 2,
            global_bits: ceil_log2(biome_count),
        }
    }

    /// How many values a container of this kind holds.
    #[must_use]
    pub const fn entry_count(self) -> usize {
        1 << (self.bits_per_axis * 3)
    }

    /// The index `Strategy.getIndex` gives a coordinate within a container.
    ///
    /// Coordinates are container-relative: 0..16 for block states, 0..4 for
    /// biomes.
    #[must_use]
    pub const fn index(self, x: u32, y: u32, z: u32) -> usize {
        (((y << self.bits_per_axis) | z) << self.bits_per_axis | x) as usize
    }

    /// Bit width the header carries for a palette of `size` entries.
    ///
    /// This is the step the wire format hides: a linear palette of two entries
    /// is written as four bits for block states, because
    /// `getConfigurationForBitCount` maps 1 through 4 onto one configuration
    /// whose `bitsInMemory` is 4.
    #[must_use]
    pub const fn bits_for_palette(self, size: usize) -> u32 {
        let needed = ceil_log2(size);
        if needed == 0 {
            return 0;
        }
        if self.bits_per_axis == 4 {
            // Strategy.createForBlockStates
            match needed {
                1..=4 => 4,
                5..=8 => needed,
                _ => self.global_bits,
            }
        } else {
            // Strategy.createForBiomes
            match needed {
                1..=3 => needed,
                _ => self.global_bits,
            }
        }
    }

    /// True when a container written at `bits` carries no palette, because
    /// its storage holds registry ids directly.
    ///
    /// The reader cannot work this out from the palette: it has to decide
    /// before reading one. `PalettedContainer.read` feeds the byte it just
    /// read back into `getConfigurationForBitCount`, so the boundary is on the
    /// written width, and it is the last non-global case in each table.
    #[must_use]
    pub const fn is_global(self, bits: u32) -> bool {
        if self.bits_per_axis == 4 {
            bits > 8
        } else {
            bits > 3
        }
    }
}

/// `Mth.ceillog2`: the number of bits needed to tell `count` values apart.
///
/// Zero for zero or one value, which is what makes a one-entry palette a
/// single-value palette with no storage at all.
const fn ceil_log2(count: usize) -> u32 {
    if count <= 1 {
        return 0;
    }
    (count - 1).bit_width()
}

/// How many `u64`s a storage of `count` values at `bits` each occupies.
///
/// `SimpleBitStorage` never lets a value straddle two longs, so the packing
/// wastes `64 % bits` bits per long. That waste is why the count is not simply
/// `count * bits / 64`.
#[must_use]
pub const fn storage_len(bits: u32, count: usize) -> usize {
    if bits == 0 {
        return 0;
    }
    let per_long = (64 / bits) as usize;
    count.div_ceil(per_long)
}

/// A palette and its bit-packed indices.
///
/// Built for encoding with [`from_values`](Self::from_values) or
/// [`single`](Self::single), and for inspection after
/// [`decode`](Self::decode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettedContainer {
    kind: ContainerKind,
    bits: u32,
    /// Registry ids in palette order, empty for a global-palette container.
    palette: Vec<i32>,
    /// Packed indices, empty when `bits` is zero.
    storage: Vec<u64>,
}

impl PalettedContainer {
    /// A container every entry of which is `id`.
    ///
    /// This is the common case by a wide margin: an empty section is all air
    /// and encodes to three bytes.
    #[must_use]
    pub fn single(kind: ContainerKind, id: i32) -> Self {
        Self {
            kind,
            bits: 0,
            palette: vec![id],
            storage: Vec::new(),
        }
    }

    /// Build a container from one registry id per entry, in
    /// [`ContainerKind::index`] order.
    ///
    /// `default` is the value the container was created with, and it takes
    /// palette id 0 whether or not it appears in `values`. That mirrors
    /// `new PalettedContainer<>(initialValue, strategy)`, whose constructor
    /// calls `palette.idFor(initialValue)` before anything is set: a section
    /// that started as air and was filled with stone keeps air in its palette.
    /// Passing the wrong default still produces a container a client reads
    /// correctly, but not the same bytes the server would have sent.
    ///
    /// The remaining entries are the distinct ids in first-seen order, which
    /// is what `LinearPalette.idFor` and `HashMapPalette.idFor` produce. Once
    /// the palette outgrows eight bits the container switches to the global
    /// palette and stores registry ids directly, as `Configuration.Global`
    /// does.
    ///
    /// # Errors
    /// Returns [`Error::NegativeLength`] when `values` is not exactly
    /// [`ContainerKind::entry_count`] long.
    pub fn from_values(kind: ContainerKind, default: i32, values: &[i32]) -> Result<Self> {
        if values.len() != kind.entry_count() {
            return Err(Error::NegativeLength(
                i32::try_from(values.len()).unwrap_or(-1),
            ));
        }

        let mut palette: Vec<i32> = vec![default];
        let mut indices: Vec<u32> = Vec::with_capacity(values.len());
        for value in values {
            let index = palette
                .iter()
                .position(|entry| entry == value)
                .unwrap_or_else(|| {
                    palette.push(*value);
                    palette.len() - 1
                });
            indices.push(u32::try_from(index).expect("palette index fits in a u32"));
        }

        let bits = kind.bits_for_palette(palette.len());
        if bits == 0 {
            return Ok(Self {
                kind,
                bits,
                palette,
                storage: Vec::new(),
            });
        }

        // Past eight bits the palette is dropped and the storage holds
        // registry ids, so the indices computed above are re-resolved.
        let global = kind.is_global(bits);
        let packed: Vec<u32> = if global {
            values
                .iter()
                .map(|id| u32::try_from(*id).unwrap_or(0))
                .collect()
        } else {
            indices
        };

        Ok(Self {
            kind,
            bits,
            palette: if global { Vec::new() } else { palette },
            storage: pack(bits, &packed),
        })
    }

    /// The bit width in the container's header.
    #[must_use]
    pub const fn bits(&self) -> u32 {
        self.bits
    }

    /// Registry ids in palette order, empty for a global-palette container.
    #[must_use]
    pub fn palette(&self) -> &[i32] {
        &self.palette
    }

    /// The registry id at `index`.
    ///
    /// # Panics
    /// Panics when `index` is past [`ContainerKind::entry_count`].
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        assert!(index < self.kind.entry_count(), "index out of range");
        if self.bits == 0 {
            return self.palette[0];
        }
        let raw = unpack(&self.storage, self.bits, index);
        if self.kind.is_global(self.bits) {
            i32::try_from(raw).unwrap_or(0)
        } else {
            usize::try_from(raw)
                .ok()
                .and_then(|index| self.palette.get(index))
                .copied()
                .unwrap_or(0)
        }
    }

    /// Bytes this container occupies, matching `getSerializedSize`.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        let palette_len = if self.bits == 0 {
            var_int_len(self.palette[0])
        } else if self.kind.is_global(self.bits) {
            0
        } else {
            var_int_len(i32::try_from(self.palette.len()).unwrap_or(i32::MAX))
                + self.palette.iter().copied().map(var_int_len).sum::<usize>()
        };
        1 + palette_len + self.storage.len() * 8
    }

    /// Read a container of `kind`.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] on truncated input or
    /// [`Error::NegativeLength`] on a palette size that cannot be a length.
    pub fn decode(kind: ContainerKind, reader: &mut Reader<'_>) -> Result<Self> {
        let bits = u32::from(reader.u8()?);

        let palette = if bits == 0 {
            vec![reader.var_int()?]
        } else if kind.is_global(bits) {
            Vec::new()
        } else {
            let size = reader.var_int()?;
            let size = usize::try_from(size).map_err(|_| Error::NegativeLength(size))?;
            // Each entry is at least one byte, so a size the frame cannot
            // supply is rejected before anything is reserved.
            if size > reader.remaining_len() {
                return Err(Error::UnexpectedEof {
                    needed: size,
                    available: reader.remaining_len(),
                });
            }
            let mut palette = Vec::with_capacity(size);
            for _ in 0..size {
                palette.push(reader.var_int()?);
            }
            palette
        };

        let longs = storage_len(bits, kind.entry_count());
        let mut storage = Vec::with_capacity(longs);
        for _ in 0..longs {
            storage.push(u64::from_ne_bytes(reader.i64()?.to_ne_bytes()));
        }

        Ok(Self {
            kind,
            bits,
            palette,
            storage,
        })
    }
}

impl Encode for PalettedContainer {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(u8::try_from(self.bits).map_err(|_| Error::NegativeLength(-1))?);

        if self.bits == 0 {
            writer.var_int(self.palette[0]);
        } else if !self.kind.is_global(self.bits) {
            writer
                .var_int(i32::try_from(self.palette.len()).map_err(|_| Error::NegativeLength(-1))?);
            for id in &self.palette {
                writer.var_int(*id);
            }
        }

        // No length prefix: `writeFixedSizeLongArray`. See the module note.
        for word in &self.storage {
            writer.i64(i64::from_ne_bytes(word.to_ne_bytes()));
        }
        Ok(())
    }
}

/// Pack `values` at `bits` each, the way `SimpleBitStorage`'s array
/// constructor does: little-endian within a long, no value straddling two.
fn pack(bits: u32, values: &[u32]) -> Vec<u64> {
    let per_long = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    let mut out = vec![0u64; values.len().div_ceil(per_long)];
    for (index, value) in values.iter().enumerate() {
        let word = index / per_long;
        // `per_long` is at most 64, so the remainder always fits in a u32.
        let shift = u32::try_from(index % per_long).expect("remainder below 64") * bits;
        out[word] |= (u64::from(*value) & mask) << shift;
    }
    out
}

/// The inverse of [`pack`] for one index.
fn unpack(storage: &[u64], bits: u32, index: usize) -> u64 {
    let per_long = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    let word = storage.get(index / per_long).copied().unwrap_or(0);
    let shift = u32::try_from(index % per_long).expect("remainder below 64") * bits;
    (word >> shift) & mask
}

/// Bytes a `VarInt` of `value` occupies (`VarInt.getByteSize`).
fn var_int_len(value: i32) -> usize {
    for index in 1..5 {
        if value & (-1i32 << (index * 7)) == 0 {
            return index;
        }
    }
    5
}
