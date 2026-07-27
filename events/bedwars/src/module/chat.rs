use flecs_ecs::{
    core::{ComponentOrPairId, EntityViewGet, SystemAPI, World, flecs},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    hyperion_minecraft_proto::{
        generated::packet_id::play::clientbound::PacketId,
        packets::play::clientbound::SystemChat,
        text::{Component, NamedColor, Style, TextColor},
    },
    net::{ConnectionId, protocol::Clientbound},
    simulation::{Name, Player, Position, event},
    storage::EventQueue,
};
use tracing::info_span;

use crate::Team;

const CHAT_COOLDOWN_SECONDS: i64 = 15; // 15 seconds
const CHAT_COOLDOWN_TICKS: i64 = CHAT_COOLDOWN_SECONDS * 20; // Convert seconds to ticks

/// The angle brackets around a speaker's name.
const BRACKET_COLOR: TextColor = TextColor::Named(NamedColor::DarkGray);

/// A style that sets the colour and inherits everything else.
fn colored(color: TextColor) -> Style<'static> {
    Style {
        color: Some(color),
        ..Style::new()
    }
}

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
                            );

                            let content = Component::text(cooldown_msg.as_str());
                            let packet = SystemChat {
                                content: content.to_tag(),
                                // False puts the line in the chat box; true
                                // would make it an action bar message.
                                overlay: false,
                            };

                            compose
                                .unicast(
                                    Clientbound::new(PacketId::SystemChat.to_raw(), &packet),
                                    *io,
                                )
                                .unwrap();
                            return;
                        }

                        cooldown.expires = current_tick + CHAT_COOLDOWN_TICKS;

                        let name = name.to_string();
                        let chat = Component::text("")
                            .append(Component::text("<").with_style(colored(BRACKET_COLOR)))
                            .append(
                                Component::text(name.as_str())
                                    .with_style(colored(TextColor::Rgb(team.rgb()))),
                            )
                            .append(Component::text("> ").with_style(colored(BRACKET_COLOR)))
                            .append(Component::text(msg));

                        let packet = SystemChat {
                            content: chat.to_tag(),
                            overlay: false,
                        };

                        let center = position.to_chunk();

                        compose
                            .broadcast_local(
                                Clientbound::new(PacketId::SystemChat.to_raw(), &packet),
                                center,
                            )
                            .send()
                            .unwrap();
                    },
                );
            }
        });
    }
}
