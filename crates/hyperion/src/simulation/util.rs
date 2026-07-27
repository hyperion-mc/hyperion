use std::sync::LazyLock;

use anyhow::{Context, bail};
use serde::Deserialize;
use valence_nbt::{Compound, Value, value::ValueRef};
use valence_registry::{
    BiomeRegistry,
    biome::{Biome, BiomeEffects},
};
use valence_server::Ident;

use crate::storage::BitStorage;

#[must_use]
pub fn registry_codec_raw() -> &'static Compound {
    static CACHED: LazyLock<Compound> = LazyLock::new(|| {
        let bytes = include_bytes!("data/registries.nbt");
        let mut bytes = &bytes[..];
        let bytes_reader = &mut bytes;
        let (compound, _) = valence_nbt::from_binary(bytes_reader).unwrap();
        compound
    });

    &CACHED
}

pub fn generate_biome_registry() -> anyhow::Result<BiomeRegistry> {
    let registry_codec = registry_codec_raw();

    // minecraft:worldgen/biome
    let biomes = registry_codec.get("minecraft:worldgen/biome").unwrap();

    let Value::Compound(biomes) = biomes else {
        bail!("expected biome to be compound");
    };

    let biomes = biomes
        .get("value")
        .context("expected biome to have value")?;

    let Value::List(biomes) = biomes else {
        bail!("expected biomes to be list");
    };

    let mut biome_registry = BiomeRegistry::default();

    for biome in biomes {
        let ValueRef::Compound(biome) = biome else {
            bail!("expected biome to be compound");
        };

        let name = biome.get("name").context("expected biome to have name")?;
        let Value::String(name) = name else {
            bail!("expected biome name to be string");
        };

        let biome = biome
            .get("element")
            .context("expected biome to have element")?;

        let Value::Compound(biome) = biome else {
            bail!("expected biome to be compound");
        };

        let biome = biome.clone();

        let downfall = biome
            .get("downfall")
            .context("expected biome to have downfall")?;
        let Value::Float(downfall) = downfall else {
            bail!("expected biome downfall to be float but is {downfall:?}");
        };

        let effects = biome
            .get("effects")
            .context("expected biome to have effects")?;
        let Value::Compound(effects) = effects else {
            bail!("expected biome effects to be compound but is {effects:?}");
        };

        let has_precipitation = biome.get("has_precipitation").with_context(|| {
            format!("expected biome biome for {name} to have has_precipitation")
        })?;
        let Value::Byte(has_precipitation) = has_precipitation else {
            bail!("expected biome biome has_precipitation to be byte but is {has_precipitation:?}");
        };
        let has_precipitation = *has_precipitation == 1;

        let temperature = biome
            .get("temperature")
            .context("expected biome to have temperature")?;
        let Value::Float(temperature) = temperature else {
            bail!("expected biome temperature to be doule but is {temperature:?}");
        };

        let effects = BiomeEffects::deserialize(effects.clone())?;

        let biome = Biome {
            downfall: *downfall,
            effects,
            has_precipitation,
            temperature: *temperature,
        };

        let ident = Ident::new(name.as_str()).unwrap();

        biome_registry.insert(ident, biome);
    }

    Ok(biome_registry)
}

/// Returns the minimum number of bits needed to represent the integer `n`.
#[must_use]
pub const fn ceil_log2(x: u32) -> u32 {
    x.bit_width()
}

/// Create a heightmap for the highest solid block at each position in the chunk.
#[must_use]
pub fn heightmap(max_height: u32, current_height: u32) -> Vec<u64> {
    let bits = ceil_log2(max_height + 1);
    let mut data = BitStorage::new(bits as usize, 16 * 16, None).unwrap();

    for x in 0_usize..16 {
        for z in 0_usize..16 {
            let index = x + z * 16;
            data.set(index, u64::from(current_height) + 1);
        }
    }

    data.into_data()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ceil_log2() {
        assert_eq!(super::ceil_log2(0), 0);
        assert_eq!(super::ceil_log2(1), 1);
        assert_eq!(super::ceil_log2(2), 2);
        assert_eq!(super::ceil_log2(3), 2);
        assert_eq!(super::ceil_log2(4), 3);
        assert_eq!(super::ceil_log2(5), 3);
        assert_eq!(super::ceil_log2(6), 3);
        assert_eq!(super::ceil_log2(7), 3);
        assert_eq!(super::ceil_log2(8), 4);
        assert_eq!(super::ceil_log2(9), 4);
        assert_eq!(super::ceil_log2(10), 4);
        assert_eq!(super::ceil_log2(11), 4);
        assert_eq!(super::ceil_log2(12), 4);
        assert_eq!(super::ceil_log2(13), 4);
        assert_eq!(super::ceil_log2(14), 4);
        assert_eq!(super::ceil_log2(15), 4);
        assert_eq!(super::ceil_log2(16), 5);
        assert_eq!(super::ceil_log2(17), 5);
        assert_eq!(super::ceil_log2(18), 5);
    }
}
