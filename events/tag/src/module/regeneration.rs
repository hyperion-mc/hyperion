use flecs_ecs::{
    core::{QueryBuilderImpl, TermBuilderImpl, World, flecs},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    Prev,
    net::Compose,
    simulation::{Player, metadata::living_entity::Health},
    util::TracingExt,
};
use tracing::info_span;

#[derive(Component)]
pub struct RegenerationModule;

#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct LastDamaged {
    pub tick: i64,
}

const MAX_HEALTH: f32 = 20.0;

impl Module for RegenerationModule {
    #[allow(clippy::excessive_nesting)]
    fn module(world: &World) {
        world.component::<LastDamaged>().meta();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, LastDamaged)>(); // todo: how does this even call Default? (IndraDb)

        system!(
            "regenerate",
            world,
            &mut LastDamaged,
            &(Prev, Health),
            &mut Health,
            &Compose($)
        )
        .multi_threaded()
        .tracing_each(
            info_span!("regenerate"),
            |(last_damaged, prev_health, health, compose)| {
                let current_tick = compose.global().tick;

                if *health < *prev_health {
                    last_damaged.tick = current_tick;
                }

                let ticks_since_damage = current_tick - last_damaged.tick;

                if health.is_dead() {
                    return;
                }

                // Calculate regeneration rate based on time since last damage
                let base_regen = 0.01; // Base regeneration per tick
                let ramp_factor = 0.0001_f32; // Increase in regeneration per tick
                let max_regen = 0.1; // Maximum regeneration per tick

                let regen_rate = ramp_factor
                    .mul_add(ticks_since_damage as f32, base_regen)
                    .min(max_regen);

                // Apply regeneration, capped at max health
                health.heal(regen_rate);
                **health = health.min(MAX_HEALTH);
            },
        );
    }
}
