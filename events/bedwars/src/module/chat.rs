use flecs_ecs::{
    core::{ComponentOrPairId, EntityViewGet, SystemAPI, World, flecs},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    net::ConnectionId,
    simulation::{Name, Player, Position, event},
    storage::EventQueue,
    valence_protocol::{
        packets::play,
        text::{Color, IntoText, Text},
    },
};
use tracing::info_span;

use crate::Team;

const CHAT_COOLDOWN_SECONDS: i64 = 15; // 15 seconds
const CHAT_COOLDOWN_TICKS: i64 = CHAT_COOLDOWN_SECONDS * 20; // Convert seconds to ticks

#[derive(Default, Component)]
#[flecs(meta)]
pub struct ChatCooldown {
    pub expires: i64,
}

#[derive(Component)]
pub struct ChatModule;

impl Module for ChatModule {
    fn module(world: &World) {
        world.component::<ChatCooldown>().meta();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, ChatCooldown)>();

        system!(
            "handle_chat_messages",
            world,
            &mut EventQueue<event::ChatMessage>,
            &hyperion::net::Compose
        )
        .each_iter(move |it, _, (event_queue, compose)| {
            let world = it.world();
            let span = info_span!("handle_chat_messages");
            let _enter = span.enter();

            let current_tick = compose.global().tick;

            for event::ChatMessage { msg, by } in event_queue.drain() {
                let msg = msg.as_str();
                let by = world.entity_from_id(by);

                // todo: we should not need this; death should occur such that this is always valid
                if !by.is_alive() {
                    continue;
                }

                // Check cooldown
                // todo: try_get if entity is dead/not found what will happen?
                by.get::<(&Name, &Position, &mut ChatCooldown, &ConnectionId, &Team)>(
                    |(name, position, cooldown, io, team)| {
                        // Check if player is still on cooldown
                        if cooldown.expires > current_tick {
                            let remaining_ticks = cooldown.expires - current_tick;
                            let remaining_secs = remaining_ticks as f32 / 20.0;

                            let cooldown_msg = format!(
                                "§cPlease wait {remaining_secs:.2} seconds before sending another \
                                 message"
                            )
                            .into_cow_text();

                            let packet = play::GameMessageS2c {
                                chat: cooldown_msg,
                                overlay: false,
                            };

                            compose.unicast(&packet, *io).unwrap();
                            return;
                        }

                        cooldown.expires = current_tick + CHAT_COOLDOWN_TICKS;

                        let chat = Text::default()
                            + "<".color(Color::DARK_GRAY)
                            + name.to_string().color(*team)
                            + "> ".color(Color::DARK_GRAY)
                            + msg.to_owned();

                        let packet = play::GameMessageS2c {
                            chat: chat.into(),
                            overlay: false,
                        };

                        let center = position.to_chunk();

                        compose.broadcast_local(&packet, center).send().unwrap();
                    },
                );
            }
        });
    }
}
