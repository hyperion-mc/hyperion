use std::fmt::Write;

use flecs_ecs::{
    core::{World, WorldGet},
    macros::Component,
    prelude::Module,
};
use hyperion::{
    net::agnostic,
    simulation::{handlers::PacketSwitchQuery, packet::HandlerRegistry},
    storage::CommandCompletionRequest,
};
use hyperion_utils::LifetimeHandle;
use valence_protocol::packets::play::CommandExecutionC2s;

use crate::component::CommandRegistry;

#[derive(Component)]
pub struct CommandSystemModule;

impl Module for CommandSystemModule {
    fn module(world: &World) {
        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |pkt: &CommandExecutionC2s<'_>,
                 _: &dyn LifetimeHandle<'_>,
                 query: &mut PacketSwitchQuery<'_>| {
                    let raw = pkt.command.0;
                    let by = query.id;
                    let Some(first_word) = raw.split_whitespace().next() else {
                        tracing::warn!("command is empty");
                        return Ok(());
                    };

                    query.world.get::<&CommandRegistry>(|command_registry| {
                        if let Some(command) = command_registry.commands.get(first_word) {
                            tracing::debug!("executing command {first_word}");

                            let command = command.on_execute;
                            command(raw, query.system, by);
                        } else {
                            tracing::debug!("command {first_word} not found");

                            let mut msg = String::new();
                            write!(&mut msg, "§cAvailable commands: §r[").unwrap();

                            for w in command_registry
                                .get_permitted(query.world, by)
                                .intersperse(", ")
                            {
                                write!(&mut msg, "{w}").unwrap();
                            }

                            write!(&mut msg, "]").unwrap();

                            let chat = agnostic::chat(msg);

                            query
                                .compose
                                .unicast(&chat, query.io_ref, query.system)
                                .unwrap();
                        }
                    });

                    Ok(())
                },
            ));
            registry.add_handler(Box::new(
                |completion: &CommandCompletionRequest<'_>,
                 _: &dyn LifetimeHandle<'_>,
                 query: &mut PacketSwitchQuery<'_>| {
                    let input = completion.query;

                    // should be in form "/{command}"
                    let command = input
                        .strip_prefix("/")
                        .unwrap_or(input)
                        .split_whitespace()
                        .next()
                        .unwrap_or("");

                    if command.is_empty() {
                        return Ok(());
                    }

                    query.world.get::<&CommandRegistry>(|registry| {
                        let Some(cmd) = registry.commands.get(command) else {
                            return;
                        };
                        let on_tab = &cmd.on_tab_complete;
                        on_tab(query, completion);
                    });

                    Ok(())
                },
            ));
        });
    }
}
