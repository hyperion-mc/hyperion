use std::{borrow::Cow, collections::BTreeMap};

use thiserror::Error;
use tracing::warn;
use valence_anvil::RegionError;
use valence_bytes::Utf8Bytes;
use valence_generated::block::{BlockKind, BlockState, PropName, PropValue};
use valence_nbt::{Compound, List, Value};
use valence_protocol::Ident;
use valence_registry::biome::BiomeId;
use valence_server::layer::chunk::{Chunk, check_biome_oob, check_block_oob, check_section_oob};

use crate::simulation::blocks::loader::parse::section::Section;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseChunkError {
    #[error("region error: {0}")]
    Region(#[from] RegionError),
    #[error("missing chunk sections")]
    MissingSections,
    #[error("missing chunk section Y")]
    MissingSectionY,
    #[error("section Y is out of bounds")]
    SectionYOutOfBounds,
    #[error("missing block states")]
    MissingBlockStates,
    #[error("missing block palette")]
    MissingBlockPalette,
    #[error("invalid block palette length")]
    BadBlockPaletteLen,
    #[error("missing block name in palette")]
    MissingBlockName,
    #[error("unknown block name of \"{0}\"")]
    UnknownBlockName(String),
    #[error("unknown property name of \"{0}\"")]
    UnknownPropName(String),
    #[error("property value of block is not a string")]
    BadPropValueType,
    #[error("unknown property value of \"{0}\"")]
    UnknownPropValue(String),
    #[error("missing packed block state data in section")]
    MissingBlockStateData,
    #[error("unexpected number of longs in block state data")]
    BadBlockLongCount,
    #[error("invalid block palette index")]
    BadBlockPaletteIndex,
    #[error("missing biomes")]
    MissingBiomes,
    #[error("missing biome palette")]
    MissingBiomePalette,
    #[error("invalid biome palette length")]
    BadBiomePaletteLen,
    #[error("biome name is not a valid resource identifier")]
    BadBiomeName,
    #[error("missing packed biome data in section")]
    MissingBiomeData,
    #[error("unexpected number of longs in biome data")]
    BadBiomeLongCount,
    #[error("invalid biome palette index")]
    BadBiomePaletteIndex,
    #[error("missing block entities")]
    MissingBlockEntities,
    #[error("missing block entity ident")]
    MissingBlockEntityIdent,
    #[error("invalid block entity ident of \"{0}\"")]
    InvalidBlockEntityName(Utf8Bytes),
    #[error("invalid block entity position")]
    InvalidBlockEntityPosition,
    #[error("missing block light")]
    MissingBlockLight,
    #[error("invalid block light")]
    InvalidBlockLight,
    #[error("missing sky light")]
    MissingSkyLight,
    #[error("invalid sky light")]
    InvalidSkyLight,
}

pub mod section;

#[derive(Clone, Default, Debug)]
pub struct ColumnData {
    pub sections: Vec<Section>,
    pub block_entities: BTreeMap<u32, Compound>,
}

impl ColumnData {
    pub fn new(height: u32) -> Self {
        Self {
            sections: vec![Section::default(); height as usize / 16],
            block_entities: BTreeMap::new(),
        }
    }

    pub fn new_with(height: u32, f: impl Fn() -> Section) -> Self {
        Self {
            sections: vec![f(); height as usize / 16],
            block_entities: BTreeMap::new(),
        }
    }

    pub fn set_delta(&mut self, x: u32, y: u32, z: u32, block: BlockState) -> BlockState {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y % 16 * 16 * 16;

        assert!(idx < 4096); // 2^12

        // todo: remove try_unwrap when we show this is safe
        let idx = u16::try_from(idx).unwrap();

        self.sections[y as usize / 16].set_delta(idx, block)
    }
}

impl Chunk for ColumnData {
    fn height(&self) -> u32 {
        u32::try_from(self.sections.len()).unwrap() * 16
    }

    fn block_state(&self, x: u32, y: u32, z: u32) -> BlockState {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y % 16 * 16 * 16;
        let state = unsafe {
            self.sections[y as usize / 16]
                .block_states
                .get_unchecked(idx as usize)
        };

        unsafe { BlockState::from_raw(state).unwrap_unchecked() }
    }

    fn set_block_state(&mut self, x: u32, y: u32, z: u32, block: BlockState) -> BlockState {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y % 16 * 16 * 16;

        assert!(idx < 4096); // 2^12

        // todo: remove try_unwrap when we show this is safe
        let idx = u16::try_from(idx).unwrap();

        self.sections[y as usize / 16].set(idx, block)
    }

    fn fill_block_state_section(&mut self, sect_y: u32, block: BlockState) {
        check_section_oob(self, sect_y);

        self.sections[sect_y as usize]
            .block_states
            .fill(block.to_raw());
    }

    fn block_entity(&self, x: u32, y: u32, z: u32) -> Option<&Compound> {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y * 16 * 16;
        self.block_entities.get(&idx)
    }

    fn block_entity_mut(&mut self, x: u32, y: u32, z: u32) -> Option<&mut Compound> {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y * 16 * 16;
        self.block_entities.get_mut(&idx)
    }

    fn set_block_entity(
        &mut self,
        x: u32,
        y: u32,
        z: u32,
        block_entity: Option<Compound>,
    ) -> Option<Compound> {
        check_block_oob(self, x, y, z);

        let idx = x + z * 16 + y * 16 * 16;

        match block_entity {
            Some(be) => self.block_entities.insert(idx, be),
            None => self.block_entities.remove(&idx),
        }
    }

    fn clear_block_entities(&mut self) {
        self.block_entities.clear();
    }

    fn biome(&self, x: u32, y: u32, z: u32) -> BiomeId {
        check_biome_oob(self, x, y, z);

        let idx = x + z * 4 + y % 4 * 4 * 4;
        self.sections[y as usize / 4].biomes.get(idx as usize)
    }

    fn set_biome(&mut self, x: u32, y: u32, z: u32, biome: BiomeId) -> BiomeId {
        check_biome_oob(self, x, y, z);

        let idx = x + z * 4 + y % 4 * 4 * 4;
        self.sections[y as usize / 4]
            .biomes
            .set(idx as usize, biome)
    }

    fn fill_biome_section(&mut self, sect_y: u32, biome: BiomeId) {
        check_section_oob(self, sect_y);

        self.sections[sect_y as usize].biomes.fill(biome);
    }

    fn shrink_to_fit(&mut self) {
        for sect in &mut self.sections {
            // sect.block_states.shrink_to_fit(); todo:
            sect.biomes.shrink_to_fit();
        }
    }
}

#[expect(clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]
pub fn parse_chunk(
    mut nbt: Compound,
    biome_map: &BTreeMap<Ident, BiomeId>, // TODO: replace with biome registry arg.
) -> Result<ColumnData, ParseChunkError> {
    let Some(Value::List(List::Compound(nbt_sections))) = nbt.remove("sections") else {
        return Err(ParseChunkError::MissingSections);
    };

    assert!(!nbt_sections.is_empty(), "empty sections");

    let mut chunk = ColumnData::new((nbt_sections.len() * 16).try_into().unwrap_or(u32::MAX));

    let min_sect_y = nbt_sections
        .iter()
        .filter_map(|sect| {
            if let Some(Value::Byte(sect_y)) = sect.get("Y") {
                Some(*sect_y)
            } else {
                None
            }
        })
        .min()
        .unwrap() as i32;

    let mut converted_block_palette = vec![];
    let mut converted_biome_palette = vec![];

    for (idx, mut section) in nbt_sections.into_iter().enumerate() {
        let Some(Value::Byte(sect_y)) = section.remove("Y") else {
            return Err(ParseChunkError::MissingSectionY);
        };

        let sect_y = (sect_y as i32 - min_sect_y) as u32;

        if sect_y >= chunk.height() / 16 {
            return Err(ParseChunkError::SectionYOutOfBounds);
        }

        match section.remove("BlockLight") {
            Some(Value::ByteArray(block_light)) => {
                let block_light = block_light.as_slice();
                let block_light: &[u8] = bytemuck::cast_slice(block_light);

                let block_light: Result<[_; 2048], _> = block_light.try_into();
                let Ok(block_light) = block_light else {
                    return Err(ParseChunkError::InvalidBlockLight);
                };

                chunk.sections[idx].block_light = Some(block_light);
            }
            Some(_) => return Err(ParseChunkError::MissingBlockLight),
            _ => {}
        }

        match section.remove("SkyLight") {
            Some(Value::ByteArray(sky_light)) => {
                let sky_light = sky_light.as_slice();
                let sky_light: &[u8] = bytemuck::cast_slice(sky_light);

                let sky_light: Result<[_; 2048], _> = sky_light.try_into();
                let Ok(sky_light) = sky_light else {
                    return Err(ParseChunkError::InvalidSkyLight);
                };

                chunk.sections[idx].sky_light = Some(sky_light);
            }
            Some(_) => return Err(ParseChunkError::MissingSkyLight),
            _ => {}
        }

        let Some(Value::Compound(mut block_states)) = section.remove("block_states") else {
            warn!("missing block states was {section:?}");
            return Err(ParseChunkError::MissingBlockStates);
        };

        let Some(Value::List(List::Compound(palette))) = block_states.remove("palette") else {
            return Err(ParseChunkError::MissingBlockPalette);
        };

        if !(1..usize::from(BLOCKS_PER_SECTION)).contains(&palette.len()) {
            return Err(ParseChunkError::BadBlockPaletteLen);
        }

        converted_block_palette.clear();

        for mut block in palette {
            let Some(Value::String(name)) = block.remove("Name") else {
                return Err(ParseChunkError::MissingBlockName);
            };

            let Some(block_kind) = BlockKind::from_str(ident_path(&name)) else {
                return Err(ParseChunkError::UnknownBlockName(name));
            };

            let mut state = block_kind.to_state();

            if let Some(Value::Compound(properties)) = block.remove("Properties") {
                for (key, value) in properties {
                    let Value::String(value) = value else {
                        return Err(ParseChunkError::BadPropValueType);
                    };

                    let Some(prop_name) = PropName::from_str(&key) else {
                        return Err(ParseChunkError::UnknownPropName(key));
                    };

                    let Some(prop_value) = PropValue::from_str(&value) else {
                        return Err(ParseChunkError::UnknownPropValue(value));
                    };

                    state = state.set(prop_name, prop_value);
                }
            }

            converted_block_palette.push(state);
        }

        if converted_block_palette.len() == 1 {
            chunk.fill_block_state_section(sect_y, converted_block_palette[0]);
        } else {
            debug_assert!(converted_block_palette.len() > 1);

            let Some(Value::LongArray(data)) = block_states.remove("data") else {
                return Err(ParseChunkError::MissingBlockStateData);
            };

            let bits_per_idx = bit_width(converted_block_palette.len() - 1).max(4);
            let idxs_per_long = 64 / bits_per_idx;
            let long_count = usize::from(BLOCKS_PER_SECTION).div_ceil(idxs_per_long);
            let mask = 2_u64.pow(u32::try_from(bits_per_idx).unwrap()) - 1;

            if long_count != data.len() {
                return Err(ParseChunkError::BadBlockLongCount);
            }

            let mut i: u32 = 0;
            for long in data {
                let long_unsigned = long as u64;

                for j in 0..idxs_per_long {
                    if i >= u32::from(BLOCKS_PER_SECTION) {
                        break;
                    }

                    let idx = (long_unsigned >> (bits_per_idx * j)) & mask;

                    let Some(block) = converted_block_palette
                        .get(usize::try_from(idx).unwrap())
                        .copied()
                    else {
                        return Err(ParseChunkError::BadBlockPaletteIndex);
                    };

                    let x = i % 16;
                    let z = i / 16 % 16;
                    let y = i / (16 * 16);

                    chunk.set_block_state(x, sect_y * 16 + y, z, block);

                    i += 1;
                }
            }
        }

        let Some(Value::Compound(biomes)) = section.get("biomes") else {
            return Err(ParseChunkError::MissingBiomes);
        };

        let Some(Value::List(List::String(palette))) = biomes.get("palette") else {
            return Err(ParseChunkError::MissingBiomePalette);
        };

        if !(1..usize::from(BIOMES_PER_SECTION)).contains(&palette.len()) {
            return Err(ParseChunkError::BadBiomePaletteLen);
        }

        converted_biome_palette.clear();

        for biome_name in palette {
            if !Ident::is_valid(&biome_name) {
                return Err(ParseChunkError::BadBiomeName);
            };

            converted_biome_palette.push(
                biome_map
                    .get(biome_name.as_str())
                    .copied()
                    .unwrap_or_default(),
            );
        }

        if converted_biome_palette.len() == 1 {
            chunk.fill_biome_section(sect_y, converted_biome_palette[0]);
        } else {
            debug_assert!(converted_biome_palette.len() > 1);

            let Some(Value::LongArray(data)) = biomes.get("data") else {
                return Err(ParseChunkError::MissingBiomeData);
            };

            let bits_per_idx = bit_width(converted_biome_palette.len() - 1);
            let idxs_per_long = 64 / bits_per_idx;
            let long_count = usize::from(BIOMES_PER_SECTION).div_ceil(idxs_per_long);
            let mask = 2_u64.pow(u32::try_from(bits_per_idx).unwrap()) - 1;

            if long_count != data.len() {
                return Err(ParseChunkError::BadBiomeLongCount);
            }

            let mut i: u32 = 0;
            for &long in data {
                let u64 = long as u64;

                for j in 0..idxs_per_long {
                    if i >= BIOMES_PER_SECTION as u32 {
                        break;
                    }

                    let idx = (u64 >> (bits_per_idx * j)) & mask;

                    let Some(biome) = converted_biome_palette
                        .get(usize::try_from(idx).unwrap())
                        .copied()
                    else {
                        return Err(ParseChunkError::BadBiomePaletteIndex);
                    };

                    let x = i % 4;
                    let z = i / 4 % 4;
                    let y = i / (4 * 4);

                    chunk.set_biome(x, sect_y * 4 + y, z, biome);

                    i += 1;
                }
            }
        }
    }

    let Some(Value::List(block_entities)) = nbt.remove("block_entities") else {
        return Err(ParseChunkError::MissingBlockEntities);
    };

    if let List::Compound(block_entities) = block_entities {
        for mut comp in block_entities {
            let Some(Value::String(ident)) = comp.remove("id") else {
                return Err(ParseChunkError::MissingBlockEntityIdent);
            };

            if let Err(e) = Ident::new(ident) {
                return Err(ParseChunkError::InvalidBlockEntityName(e.0));
            }

            let Some(Value::Int(x)) = comp.remove("x") else {
                return Err(ParseChunkError::InvalidBlockEntityPosition);
            };

            let x = x.rem_euclid(16) as u32;

            let Some(Value::Int(y)) = comp.remove("y") else {
                return Err(ParseChunkError::InvalidBlockEntityPosition);
            };

            let Ok(y) = u32::try_from(y.wrapping_sub(min_sect_y * 16)) else {
                return Err(ParseChunkError::InvalidBlockEntityPosition);
            };

            if y >= chunk.height() {
                return Err(ParseChunkError::InvalidBlockEntityPosition);
            }

            let Some(Value::Int(z)) = comp.remove("z") else {
                return Err(ParseChunkError::InvalidBlockEntityPosition);
            };

            let z = z.rem_euclid(16) as u32;

            comp.remove("keepPacked");

            chunk.set_block_entity(x, y, z, Some(comp));
        }
    }

    Ok(chunk)
}

const BLOCKS_PER_SECTION: u16 = 16 * 16 * 16;
const BIOMES_PER_SECTION: u16 = 4 * 4 * 4;

/// Gets the path part of a resource identifier.
fn ident_path(ident: &str) -> &str {
    match ident.rsplit_once(':') {
        Some((_, after)) => after,
        None => ident,
    }
}

/// Returns the minimum number of bits needed to represent the integer `n`.
const fn bit_width(n: usize) -> usize {
    (usize::BITS - n.leading_zeros()) as _
}
