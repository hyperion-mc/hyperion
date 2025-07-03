mod cached_save;
pub mod iterator;
pub mod prev;
use std::path::PathBuf;

use bevy::{
    ecs::system::{SystemParam, SystemState},
    prelude::*,
};
pub use cached_save::cached_save;
pub use prev::{Prev, track_prev};

pub trait EntityExt: Sized {
    fn minecraft_id(&self) -> i32;
    fn from_minecraft_id(id: i32, world: &World) -> anyhow::Result<Self>;
}

impl EntityExt for Entity {
    fn minecraft_id(&self) -> i32 {
        let index = self.index();
        bytemuck::cast(index)
    }

    fn from_minecraft_id(id: i32, world: &World) -> anyhow::Result<Self> {
        let id: u32 = bytemuck::cast(id);

        // TODO: According to the docs, this should check if the returned entity is freed
        world
            .entities()
            .resolve_from_id(id)
            .ok_or_else(|| anyhow::anyhow!("minecraft id is invalid"))
    }
}

pub trait ApplyWorld {
    fn apply(&mut self, world: &mut World);
}

impl<Param> ApplyWorld for SystemState<Param>
where
    Param: SystemParam + 'static,
{
    fn apply(&mut self, world: &mut World) {
        self.apply(world);
    }
}

impl ApplyWorld for () {
    fn apply(&mut self, _: &mut World) {}
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

pub struct HyperionUtilsPlugin;

impl Plugin for HyperionUtilsPlugin {
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
        let mut world = World::new();
        let entity_id = world.spawn_empty().id();
        let minecraft_id = entity_id.minecraft_id();
        assert_eq!(
            Entity::from_minecraft_id(minecraft_id, &world).unwrap(),
            entity_id
        );
    }
}
