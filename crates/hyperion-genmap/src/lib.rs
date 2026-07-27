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

        world.get::<&AsyncRuntime>(|runtime| {
            const URL: &str = "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";

            let f = hyperion_utils::cached_save(world, URL);

            let save = runtime.block_on(f).unwrap_or_else(|e| {
                panic!("failed to download map {URL}: {e}");
            });

            world.set(Blocks::new(world, &save).unwrap());
        });
    }
}
