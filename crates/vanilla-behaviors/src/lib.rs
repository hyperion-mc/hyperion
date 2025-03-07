use flecs_ecs::core::EntityView;
use hyperion::{
    flecs_ecs::{self, core::EntityViewGet},
    net::Compose,
    simulation::{metadata::living_entity::Health, LastDamaged, Player, Position},
};
use hyperion_utils::{structures::DamageCause, EntityExt};
use tracing::warn;
use valence_protocol::{packets::play, VarInt};
use valence_server::GameMode;

pub mod command;
pub mod module;

#[must_use]
pub const fn is_invincible(gamemode: &GameMode) -> bool {
    matches!(gamemode, GameMode::Creative | GameMode::Spectator)
}

pub fn damage_player(
    entity: &EntityView<'_>,
    amount: f32,
    damage_cause: DamageCause,
    compose: &Compose,
    system: EntityView<'_>,
) -> bool {
    if entity.has::<Player>() {
        entity.get::<(&mut Health, &mut LastDamaged, &Position)>(|(health, last_damaged, pos)| {
            if !health.is_dead() {
                let mut applied_damages = 0.;

                if compose.global().tick - last_damaged.tick >= 10 {
                    applied_damages = amount;
                    last_damaged.tick = compose.global().tick;
                } else if compose.global().tick - last_damaged.tick < 10
                    && last_damaged.amount < amount
                {
                    applied_damages = amount - last_damaged.amount;
                }

                if applied_damages > 0. {
                    last_damaged.amount = amount;
                    health.damage(applied_damages);

                    let pkt_damage_event = play::EntityDamageS2c {
                        entity_id: VarInt(entity.minecraft_id()),
                        source_type_id: VarInt(damage_cause.damage_type as i32),
                        source_cause_id: VarInt(damage_cause.source_entity + 1),
                        source_direct_id: VarInt(damage_cause.direct_source + 1),
                        source_pos: damage_cause.position,
                    };

                    if compose
                        .broadcast_local(&pkt_damage_event, pos.to_chunk(), system)
                        .send()
                        .is_err()
                    {
                        warn!("Failed to brodcast EntityDamageS2c locally!");
                    }
                    return true;
                }
            }
            false
        })
    } else {
        warn!("Trying to call a Player only function on an non player entity");
        false
    }
}
