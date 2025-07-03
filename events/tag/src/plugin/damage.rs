use bevy::prelude::*;
use hyperion::{
    net::{Compose, ConnectionId, agnostic},
    simulation::{Position, event::HitGroundEvent, metadata::living_entity::Health},
};
use hyperion_utils::EntityExt;
use tracing::error;
use valence_protocol::{VarInt, packets::play, text::IntoText};
use valence_server::ident;

fn apply_natural_damages(
    mut events: EventReader<'_, '_, HitGroundEvent>,
    mut query: Query<'_, '_, (&mut Health, &ConnectionId, &Position)>,
    compose: Res<'_, Compose>,
) {
    for event in events.read() {
        if event.fall_distance <= 3. {
            continue;
        }

        // TODO account for armor/effects and gamemode
        let damage = event.fall_distance.floor() - 3.;

        if damage <= 0. {
            continue;
        }

        let (mut health, &connection_id, position) = match query.get_mut(event.client) {
            Ok(data) => data,
            Err(e) => {
                error!("failed to apply natural damages: query failed: {e}");
                continue;
            }
        };

        health.damage(damage);

        let pkt_damage_event = play::EntityDamageS2c {
            entity_id: VarInt(event.client.minecraft_id()),
            source_cause_id: VarInt(0),
            source_direct_id: VarInt(0),
            source_type_id: VarInt(10), // 10 = fall damage
            source_pos: Option::None,
        };

        let sound = agnostic::sound(
            if event.fall_distance > 7. {
                ident!("minecraft:entity.player.big_fall")
            } else {
                ident!("minecraft:entity.player.small_fall")
            },
            **position,
        )
        .volume(1.)
        .pitch(1.)
        .seed(fastrand::i64(..))
        .build();

        compose.unicast(&pkt_damage_event, connection_id).unwrap();
        compose
            .broadcast_local(&sound, position.to_chunk())
            .send()
            .unwrap();

        if health.is_dead() {
            let pkt_death_screen = play::DeathMessageS2c {
                player_id: VarInt(event.client.minecraft_id()),
                message: (if event.fall_distance < 5.0 {
                    "You hit the ground too hard"
                } else {
                    "You fell from a high place"
                })
                .to_string()
                .into_cow_text(),
            };
            compose.unicast(&pkt_death_screen, connection_id).unwrap();
        }
    }
}

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, apply_natural_damages);
    }
}
