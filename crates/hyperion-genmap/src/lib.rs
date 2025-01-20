use std::path::PathBuf;

use flecs_ecs::{
    core::{World, WorldGet},
    macros::Component,
    prelude::Module,
};
use hyperion::{runtime::AsyncRuntime, simulation::blocks::Blocks};

#[derive(Component)]
pub struct GenMapModule;

impl Module for GenMapModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world.import::<hyperion_utils::HyperionUtilsModule>();

        let save_path = std::env::var("HYPERION_GENMAP_PATH").map_or_else(
            |_| {
                world.get::<&AsyncRuntime>(|runtime| {
                    const URL: &str =
                        "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";
                    let f = hyperion_utils::cached_save(world, URL);

                    runtime.block_on(f).unwrap_or_else(|e| {
                        panic!("failed to download map {URL}: {e}");
                    })
                })
            },
            PathBuf::from,
        );

        world.set(Blocks::new(world, &save_path).unwrap());
    }
}
