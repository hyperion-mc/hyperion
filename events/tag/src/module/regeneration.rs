use flecs_ecs::{
    core::{QueryBuilderImpl, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    Prev,
    net::Compose,
    simulation::{LastDamaged, metadata::living_entity::Health},
    util::TracingExt,
};
use tracing::info_span;

#[derive(Component)]
pub struct RegenerationModule;

const MAX_HEALTH: f32 = 20.0;

impl Module for RegenerationModule {
    #[allow(clippy::excessive_nesting)]
    fn module(world: &World) {
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
