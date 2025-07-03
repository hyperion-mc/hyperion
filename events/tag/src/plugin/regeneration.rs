use bevy::prelude::*;
use hyperion::{
    net::Compose,
    simulation::{metadata::living_entity::Health, packet_state},
};
use hyperion_utils::Prev;

const MAX_HEALTH: f32 = 20.0;

pub struct RegenerationPlugin;

#[derive(Component, Default, Copy, Clone, Debug)]
pub struct LastDamaged {
    pub tick: i64,
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert(LastDamaged::default());
}

fn regenerate(
    query: Query<'_, '_, (&mut LastDamaged, &Prev<Health>, &mut Health)>,
    compose: Res<'_, Compose>,
) {
    let current_tick = compose.global().tick;

    for (mut last_damaged, prev_health, mut health) in query {
        if *health < **prev_health {
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
    }
}

impl Plugin for RegenerationPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_player);
        app.add_systems(FixedPostUpdate, regenerate);
    }
}
