use flecs_ecs::{
    core::{EntityViewGet, World, WorldGet, flecs},
    macros::Component,
    prelude::Module,
};
use hyperion::{
    simulation::{Name, Player, handlers::PacketSwitchQuery, packet::HandlerRegistry},
    valence_protocol::{
        packets::play::{self, ChatMessageC2s},
        text::IntoText,
    },
};
use hyperion_rank_tree::Team;
use tracing::info_span;

const CHAT_COOLDOWN_SECONDS: i64 = 15; // 15 seconds
const CHAT_COOLDOWN_TICKS: i64 = CHAT_COOLDOWN_SECONDS * 20; // Convert seconds to ticks

#[derive(Default, Component)]
#[meta]
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

        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |packet: &ChatMessageC2s<'_>, query: &mut PacketSwitchQuery<'_>| {
                    let span = info_span!("handle_chat_messages");
                    let _enter = span.enter();

                    let current_tick = query.compose.global().tick;
                    let by = query.view;

                    // todo: we should not need this; death should occur such that this is always valid
                    if !by.is_alive() {
                        return Ok(());
                    }

                    // Check cooldown
                    // todo: try_get if entity is dead/not found what will happen?
                    by.get::<(&Name, &mut ChatCooldown, &Team)>(|(name, cooldown, team)| {
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

                            query
                                .compose
                                .unicast(&packet, query.io_ref, query.system)
                                .unwrap();
                            return;
                        }

                        cooldown.expires = current_tick + CHAT_COOLDOWN_TICKS;

                        let color_prefix = match team {
                            Team::Blue => "§9",
                            Team::Green => "§a",
                            Team::Red => "§c",
                            Team::Yellow => "§e",
                        };

                        let msg = packet.message;

                        let chat = format!("§8<{color_prefix}{name}§8>§r {msg}").into_cow_text();
                        let packet = play::GameMessageS2c {
                            chat,
                            overlay: false,
                        };

                        let center = query.position.to_chunk();

                        query
                            .compose
                            .broadcast_local(&packet, center, query.system)
                            .send()
                            .unwrap();
                    });

                    Ok(())
                },
            ));
        });
    }
}
