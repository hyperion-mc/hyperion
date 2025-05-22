mod cached_save;
mod lifetime;
use std::path::PathBuf;

use bevy::prelude::*;
pub use cached_save::cached_save;
pub use lifetime::*;

pub trait EntityExt: Sized {
    fn minecraft_id(&self) -> anyhow::Result<i32>;
    fn from_minecraft_id(id: i32) -> anyhow::Result<Self>;
}

// todo(bevy): this is probably different with bevy

// we need a mapping from Bevy entity id to Minecraft entity id
// some
impl EntityExt for Entity {
    fn minecraft_id(&self) -> anyhow::Result<i32> {
        let index = self.index();
        let generation = self.generation();

        let index_u16 = u16::try_from(index).unwrap();

        let generation_u16 = u16::try_from(generation).unwrap();

        let index_u32 = u32::from(index_u16);
        let generation_u32 = u32::from(generation_u16);

        let raw_u32 = index_u32 << 16 | generation_u32;

        Ok(bytemuck::cast(raw_u32))
    }

    fn from_minecraft_id(id: i32) -> anyhow::Result<Self> {
        let id: u32 = bytemuck::cast(id);

        let index = id >> 16;
        let generation = id & 0xFFFF;

        let raw_u64 = u64::from(index) | (u64::from(generation) << 32);
        Ok(Self::from_bits(raw_u64))
    }
}

/// Represents application identification information used for caching and other system-level operations
#[derive(Resource)]
pub struct AppId {
    /// The qualifier/category of the application (e.g. "com", "org", "hyperion")
    pub qualifier: String,
    /// The organization that created the application (e.g. "andrewgazelka")
    pub organization: String,
    /// The specific application name (e.g. "proof-of-concept")
    pub application: String,
}

impl AppId {
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        let project_dirs = directories::ProjectDirs::from(
            self.qualifier.as_str(),
            self.organization.as_str(),
            self.application.as_str(),
        )
        .unwrap();
        project_dirs.cache_dir().to_path_buf()
    }
}

#[derive(Component)]
pub struct HyperionUtilsModule;

impl Plugin for HyperionUtilsModule {
    fn build(&self, app: &mut App) {
        app.insert_resource(AppId {
            qualifier: "github".to_string(),
            organization: "hyperion-mc".to_string(),
            application: "generic".to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_id() {
        let entity = Entity::from_raw(0xDEAD_BEEF);
        let id = entity.minecraft_id().unwrap();
        assert_eq!(id, 0xDEAD_BEEF);

        let entity = Entity::from_raw(0xDEAD_BEEF);
        let id = entity.minecraft_id().unwrap();
        assert_eq!(id, 0xDEAD_BEEF);
    }
}
