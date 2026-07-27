use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use tokio::runtime::Runtime;
use valence_protocol::Ident;
use valence_registry::{BiomeRegistry, biome::BiomeId};

use super::{manager::RegionManager, translate};

/// Inner state of the [`MinecraftWorld`] component.
pub struct WorldShared {
    pub regions: RegionManager,
    /// Anvil biome name to the id protocol 776 sends.
    ///
    /// The ids are already translated, so nothing downstream of chunk parsing
    /// has to know that the world was written by a different game version.
    /// See [`translate::biome_name_to_id`].
    pub biome_to_id: BTreeMap<Ident, BiomeId>,
}

impl WorldShared {
    pub(crate) fn new(
        biomes: &BiomeRegistry,
        runtime: &Runtime,
        path: &Path,
    ) -> anyhow::Result<Self> {
        let regions = RegionManager::new(runtime, path).context("failed to get anvil data")?;

        let biome_to_id = translate::biome_name_to_id(biomes);

        Ok(Self {
            regions,
            biome_to_id,
        })
    }
}
