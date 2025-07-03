use bevy::prelude::*;
use hyperion::{runtime::AsyncRuntime, simulation::blocks::Blocks};

pub struct GenMapPlugin;

impl Plugin for GenMapPlugin {
    fn build(&self, app: &mut App) {
        const URL: &str = "https://github.com/andrewgazelka/maps/raw/main/GenMap.tar.gz";

        let runtime = app
            .world()
            .get_resource::<AsyncRuntime>()
            .expect("AsyncRuntime resource must exist");
        let f = hyperion_utils::cached_save(app.world(), URL);

        let save = runtime.block_on(f).unwrap_or_else(|e| {
            panic!("failed to download map {URL}: {e}");
        });

        app.insert_resource(Blocks::new(runtime, &save).unwrap());
    }
}
