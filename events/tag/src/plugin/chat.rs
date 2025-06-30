use bevy::prelude::*;
use hyperion::{
    ingress,
    net::{Compose, ConnectionId},
    simulation::{Position, packet, packet_state},
    valence_protocol::{packets::play, text::IntoText},
};
use hyperion_rank_tree::Team;
use tracing::error;

const CHAT_COOLDOWN_SECONDS: i64 = 3; // 3 seconds
const CHAT_COOLDOWN_TICKS: i64 = CHAT_COOLDOWN_SECONDS * 20; // Convert seconds to ticks

#[derive(Default, Component)]
pub struct ChatCooldown {
    pub expires: i64,
}

pub fn initialize_cooldown(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert(ChatCooldown::default());
}

pub fn handle_chat_messages(
    mut packets: EventReader<'_, '_, packet::play::ChatMessage>,
    compose: Res<'_, Compose>,
    mut query: Query<'_, '_, (&Name, &Position, &mut ChatCooldown, &ConnectionId, &Team)>,
) {
    let current_tick = compose.global().tick;

    for packet in packets.read() {
        let (name, position, mut cooldown, io, team) = match query.get_mut(packet.sender()) {
            Ok(data) => data,
            Err(e) => {
                error!("could not process chat message: query failed: {e}");
                continue;
            }
        };

        // Check if player is still on cooldown
        if cooldown.expires > current_tick {
            let remaining_ticks = cooldown.expires - current_tick;
            let remaining_secs = remaining_ticks as f32 / 20.0;

            let cooldown_msg =
                format!("§cPlease wait {remaining_secs:.2} seconds before sending another message")
                    .into_cow_text();

            let packet = play::GameMessageS2c {
                chat: cooldown_msg,
                overlay: false,
            };

            compose.unicast(&packet, *io).unwrap();
            continue;
        }

        cooldown.expires = current_tick + CHAT_COOLDOWN_TICKS;

        let color_prefix = match team {
            Team::Blue => "§9",
            Team::Green => "§a",
            Team::Red => "§c",
            Team::Yellow => "§e",
        };

        let chat = format!("§8<{color_prefix}{name}§8>§r {}", &packet.message).into_cow_text();
        let packet = play::GameMessageS2c {
            chat,
            overlay: false,
        };

        let center = position.to_chunk();

        compose.broadcast_local(&packet, center).send().unwrap();
    }
}

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_cooldown);
        app.add_systems(
            FixedUpdate,
            handle_chat_messages.after(ingress::decode::play),
        );
    }
}
