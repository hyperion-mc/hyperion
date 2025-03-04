use flecs_ecs::core::EntityView;
use hyperion::{
    flecs_ecs::{self, core::EntityViewGet},
    net::Compose,
    simulation::{metadata::living_entity::Health, LastDamaged, Player},
};
use tracing::warn;

pub mod command;
pub mod module;

pub fn damage_player(entity: &EntityView<'_>, amount: f32, compose: &Compose) {
    if entity.has::<Player>() {
        entity.get::<(&mut Health, &mut LastDamaged)>(|(health, last_damaged)| {
            if !health.is_dead() {
                health.damage(amount);
                last_damaged.tick = compose.global().tick;
            }
        });
    } else {
        warn!("Trying to call a Player only function on an non player entity");
    }
}
