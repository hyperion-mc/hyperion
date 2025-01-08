use std::fmt::Write;

use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World, WorldGet},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    net::agnostic,
    simulation::{event, handlers::PacketSwitchQuery, packet::HandlerRegistry},
    storage::{CommandCompletionRequest, EventQueue},
};
use hyperion_utils::LifetimeHandle;

use crate::component::CommandRegistry;

#[derive(Component)]
pub struct CommandSystemModule;

impl Module for CommandSystemModule {
    fn module(world: &World) {
        system!(
            "execute_command",
            world,
            &mut EventQueue<event::Command>($),
            &CommandRegistry($)
        )
        .each_iter(|it, _, (event_queue, registry)| {
            let system = it.system();

            let world = it.world();
            for event::Command { raw, by } in event_queue.drain() {
                let raw = raw.get();
                let Some(first_word) = raw.split_whitespace().next() else {
                    tracing::warn!("command is empty");
                    continue;
                };

                let Some(command) = registry.commands.get(first_word) else {
                    tracing::debug!("command {first_word} not found");

                    let mut msg = String::new();
                    write!(&mut msg, "§cAvailable commands: §r[").unwrap();

                    for w in registry.get_permitted(&world, by).intersperse(", ") {
                        write!(&mut msg, "{w}").unwrap();
                    }

                    write!(&mut msg, "]").unwrap();

                    let chat = agnostic::chat(msg);

                    world.get::<&hyperion::net::Compose>(|compose| {
                        by.entity_view(world)
                            .get::<&hyperion::net::ConnectionId>(|stream| {
                                compose.unicast(&chat, *stream, system).unwrap();
                            });
                    });

                    continue;
                };

                tracing::debug!("executing command {first_word}");

                let command = command.on_execute;
                command(raw, system, by);
            }
        });

        world.get::<&mut HandlerRegistry>(|registry| {
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
