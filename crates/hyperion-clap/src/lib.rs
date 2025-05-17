use std::iter::zip;

use clap::{Arg as ClapArg, Parser, ValueEnum, ValueHint, error::ErrorKind};
use flecs_ecs::{
    core::{Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider, id},
    prelude::{Component, Module},
};
use hyperion::{
    net::{Compose, ConnectionId, DataBundle, agnostic},
    simulation::{IgnMap, command::get_root_command_entity, handlers::PacketSwitchQuery},
    storage::CommandCompletionRequest,
};
pub use hyperion_clap_macros::CommandPermission;
pub use hyperion_command;
use hyperion_command::{CommandHandler, CommandRegistry};
use hyperion_permission::Group;
use valence_protocol::{
    VarInt,
    packets::{
        play,
        play::{command_suggestions_s2c::CommandSuggestionsMatch, command_tree_s2c::StringArg},
    },
};

pub trait MinecraftCommand: Parser + CommandPermission {
    fn execute(self, system: EntityView<'_>, caller: Entity);

    fn pre_register(_world: &World) {}

    fn register(registry: &mut CommandRegistry, world: &World) {
        Self::pre_register(world);

        let cmd = Self::command();
        let name = cmd.get_name();

        let has_permissions = |world: &World, caller: Entity| {
            caller
                .entity_view(world)
                .get::<&Group>(|group| Self::has_required_permission(*group))
        };

        let node_to_register =
            hyperion::simulation::command::Command::literal(name, has_permissions);

        let mut on = world
            .entity()
            .set(node_to_register)
            .child_of(get_root_command_entity());

        for arg in cmd.get_arguments() {
            use valence_protocol::packets::play::command_tree_s2c::Parser as ValenceParser;
            let name = arg.get_value_names().unwrap().first().unwrap();
            let name = name.to_ascii_lowercase();
            let node_to_register = hyperion::simulation::command::Command::argument(
                name,
                ValenceParser::String(StringArg::SingleWord),
            );

            on = world.entity().set(node_to_register).child_of(on);
        }

        let on_execute = |input: &str, system: EntityView<'_>, caller: Entity| {
            let input = input.split_whitespace();
            let world = system.world();

            match Self::try_parse_from(input) {
                Ok(elem) => {
                    if world.get::<&Compose>(|compose| {
                        caller.entity_view(world).get::<(&ConnectionId, &Group)>(
                            |(stream, group)| {
                                if Self::has_required_permission(*group) {
                                    true
                                } else {
                                    let chat = agnostic::chat(
                                        "§cYou do not have permission to use this command!",
                                    );

                                    let mut bundle = DataBundle::new(compose, system);
                                    bundle.add_packet(&chat).unwrap();
                                    bundle.unicast(*stream).unwrap();

                                    false
                                }
                            },
                        )
                    }) {
                        elem.execute(system, caller);
                    }
                }
                Err(e) => {
                    // add red if not display help
                    let prefix = match e.kind() {
                        ErrorKind::DisplayHelp => "",
                        _ => "§c",
                    };

                    // minecraft red
                    let msg = format!("{prefix}{e}");

                    world.get::<&Compose>(|compose| {
                        caller
                            .entity_view(world)
                            .get::<&hyperion::net::ConnectionId>(|stream| {
                                let msg = agnostic::chat(msg);
                                compose.unicast(&msg, *stream, system).unwrap();
                            });
                    });

                    tracing::warn!("could not parse command {e}");
                }
            }
        };

        let on_tab_complete = Box::new(
            |packet_switch_query: &mut PacketSwitchQuery<'_>,
             completion: &CommandCompletionRequest<'_>| {
                let full_query = completion.query;
                let id = completion.id;

                let Some(query) = full_query.strip_prefix('/') else {
                    // todo: send error message to player
                    tracing::warn!("could not parse command {full_query}");
                    return;
                };

                let mut query = query.split_whitespace();
                let _command_name = query.next().unwrap();

                let command = Self::command();
                let mut positionals = command.get_positionals();

                'positionals: for (input_arg, cmd_arg) in zip(query, positionals.by_ref()) {
                    // see if anything matches
                    let possible_values = cmd_arg.get_possible_values();
                    for possible in &possible_values {
                        if possible.matches(input_arg, true) {
                            continue 'positionals;
                        }
                    }

                    // nothing matches! let's see if a substring matches
                    let mut substring_matches = possible_values
                        .iter()
                        .filter(|possible| {
                            // todo: this is inefficient
                            possible
                                .get_name()
                                .to_lowercase()
                                .starts_with(&input_arg.to_lowercase())
                        })
                        .peekable();

                    if substring_matches.peek().is_none() {
                        // no matches
                        return;
                    }

                    let matches = substring_matches
                        .map(clap::builder::PossibleValue::get_name)
                        .map(|name| CommandSuggestionsMatch {
                            suggested_match: name,
                            tooltip: None,
                        })
                        .collect();

                    let start = input_arg.as_ptr() as usize - full_query.as_ptr() as usize;
                    let len = input_arg.len();

                    let start = i32::try_from(start).unwrap();
                    let len = i32::try_from(len).unwrap();

                    let packet = play::CommandSuggestionsS2c {
                        id: VarInt(id),
                        start: VarInt(start),
                        length: VarInt(len),
                        matches,
                    };

                    packet_switch_query
                        .compose
                        .unicast(
                            &packet,
                            packet_switch_query.io_ref,
                            packet_switch_query.system,
                        )
                        .unwrap();

                    // todo: send possible matches to player
                    return;
                }

                let Some(remaining_positional) = positionals.next() else {
                    // we are all done completing
                    return;
                };

                let possible_values = remaining_positional.get_possible_values();

                let names = possible_values
                    .iter()
                    .map(clap::builder::PossibleValue::get_name);

                let matches = names
                    .into_iter()
                    .map(|name| CommandSuggestionsMatch {
                        suggested_match: name,
                        tooltip: None,
                    })
                    .collect();

                let start = full_query.len();
                let start = i32::try_from(start).unwrap();

                let packet = play::CommandSuggestionsS2c {
                    id: VarInt(id),
                    start: VarInt(start),
                    length: VarInt(0),
                    matches,
                };

                packet_switch_query
                    .compose
                    .unicast(
                        &packet,
                        packet_switch_query.io_ref,
                        packet_switch_query.system,
                    )
                    .unwrap();
            },
        );

        let handler = CommandHandler {
            on_execute,
            on_tab_complete,
            has_permissions,
        };

        tracing::info!("registering command {name}");

        registry.register(name, handler);
    }
}

pub enum Arg {
    Player,
}

// Custom trait for Minecraft-specific argument behavior
pub trait MinecraftArg {
    #[must_use]
    fn minecraft(self, parser: Arg) -> Self;
}

// Implement the trait for Arg
impl MinecraftArg for ClapArg {
    fn minecraft(self, arg: Arg) -> Self {
        match arg {
            Arg::Player => self.value_hint(ValueHint::Username),
        }
    }
}

pub trait CommandPermission {
    fn has_required_permission(user_group: hyperion_permission::Group) -> bool;
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

#[derive(Component)]
pub struct ClapCommandModule;

#[derive(clap::Parser, Debug)]
pub struct SetCommand {
    player: String,
    group: Group,
}

#[derive(clap::Parser, Debug)]
pub struct GetCommand {
    player: String,
}

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "perms")]
#[command_permission(group = "Normal")]
pub enum PermissionCommand {
    Set(SetCommand),
    Get(GetCommand),
}

impl MinecraftCommand for PermissionCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        world.get::<&IgnMap>(|ign_map| {
            match self {
                Self::Set(cmd) => {
                    // Handle setting permissions
                    let Some(entity) = ign_map.get(cmd.player.as_str()) else {
                        caller.entity_view(world).get::<&ConnectionId>(|stream| {
                            let msg = format!("§c{} not found", cmd.player);
                            let chat = hyperion::net::agnostic::chat(msg);
                            world.get::<&Compose>(|compose| {
                                compose.unicast(&chat, *stream, system).unwrap();
                            });
                        });
                        return;
                    };

                    entity.entity_view(world).get::<&mut Group>(|group| {
                        if *group != cmd.group {
                            *group = cmd.group;
                            entity.entity_view(world).modified(id::<Group>());
                        }

                        caller.entity_view(world).get::<&ConnectionId>(|stream| {
                            let msg = format!(
                                "§b{}§r's group has been set to §e{:?}",
                                cmd.player, cmd.group
                            );
                            let chat = hyperion::net::agnostic::chat(msg);
                            world.get::<&Compose>(|compose| {
                                compose.unicast(&chat, *stream, system).unwrap();
                            });
                        });
                    });
                }
                Self::Get(cmd) => {
                    let Some(entity) = ign_map.get(cmd.player.as_str()) else {
                        caller.entity_view(world).get::<&ConnectionId>(|stream| {
                            let msg = format!("§c{} not found", cmd.player);
                            let chat = hyperion::net::agnostic::chat(msg);
                            world.get::<&Compose>(|compose| {
                                compose.unicast(&chat, *stream, system).unwrap();
                            });
                        });
                        return;
                    };

                    entity.entity_view(world).get::<&Group>(|group| {
                        caller.entity_view(world).get::<&ConnectionId>(|stream| {
                            let msg = format!("§b{}§r's group is §e{:?}", cmd.player, group);
                            let chat = hyperion::net::agnostic::chat(msg);
                            world.get::<&Compose>(|compose| {
                                compose.unicast(&chat, *stream, system).unwrap();
                            });
                        });
                    });
                }
            }
        });
    }
}

impl Module for ClapCommandModule {
    fn module(world: &World) {
        world.import::<hyperion_command::CommandModule>();

        world.get::<&mut CommandRegistry>(|registry| {
            PermissionCommand::register(registry, world);
        });
    }
}
