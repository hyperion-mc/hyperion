use std::iter::zip;

use bevy::{ecs::system::SystemState, prelude::*};
use clap::{Arg as ClapArg, Parser, ValueEnum, ValueHint, error::ErrorKind};
use hyperion::{
    net::{Compose, ConnectionId, DataBundle, agnostic},
    simulation::{IgnMap, command::RootCommand, packet::play},
};
pub use hyperion_clap_macros::CommandPermission;
pub use hyperion_command;
use hyperion_command::{CommandHandler, CommandRegistry, ExecutableCommand};
use hyperion_permission::Group;
use hyperion_utils::ApplyWorld;
use tracing::error;
use valence_bytes::Utf8Bytes;
use valence_protocol::{
    VarInt,
    packets::play::{
        command_suggestions_s2c::{CommandSuggestionsMatch, CommandSuggestionsS2c},
        command_tree_s2c::StringArg,
    },
};

struct GenericExecutableCommand<Command: MinecraftCommand> {
    state: Command::State,
}

impl<Command: MinecraftCommand> ExecutableCommand for GenericExecutableCommand<Command> {
    fn execute(&mut self, world: &World, packet: &play::CommandExecution) {
        let compose = world.resource::<Compose>();
        let input = packet.command.split_whitespace();

        match Command::try_parse_from(input) {
            Ok(elem) => {
                let Some(group) = world.entity(packet.sender()).get::<Group>() else {
                    error!("failed to execute command: player is missing Group component");
                    return;
                };

                if Command::has_required_permission(*group) {
                    elem.execute(world, &mut self.state, packet.sender());
                } else {
                    let chat = agnostic::chat("§cYou do not have permission to use this command!");

                    let mut bundle = DataBundle::new(compose);
                    bundle.add_packet(&chat).unwrap();
                    bundle.unicast(packet.connection_id()).unwrap();
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

                let msg = agnostic::chat(msg);
                compose.unicast(&msg, packet.connection_id()).unwrap();

                tracing::warn!("could not parse command {e}");
            }
        }
    }
}

impl<Command: MinecraftCommand> ApplyWorld for GenericExecutableCommand<Command> {
    fn apply(&mut self, world: &mut World) {
        self.state.apply(world);
    }
}

pub trait MinecraftCommand: Parser + CommandPermission + 'static {
    /// Command state passed to [`MinecraftCommand::execute`]. This can be any type, but it may be
    /// useful to store a [`bevy::ecs::system::SystemState`] to access types implementing
    /// [`bevy::ecs::system::ReadOnlySystemParam`]. The state is initialized using the [`FromWorld`]
    /// trait, and it is applied with the [`ApplyWorld`] trait once every `FixedMain`.
    type State: FromWorld + ApplyWorld + Send + Sync + 'static;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity);

    fn pre_register(_world: &World) {}

    fn register(world: &mut World) {
        Self::pre_register(world);

        let state = Self::State::from_world(world);

        let cmd = Self::command();
        let name = Utf8Bytes::copy_from_str(cmd.get_name());

        let has_permissions = |world: &World, caller: Entity| {
            let Some(group) = world.entity(caller).get::<Group>() else {
                error!("failed to check command permissions: client is missing Group component");
                return false;
            };

            Self::has_required_permission(*group)
        };

        let node_to_register =
            hyperion::simulation::command::Command::literal(name.clone(), has_permissions);

        let root_command = world.resource::<RootCommand>();
        let mut on = **root_command;
        on = world.spawn((node_to_register, ChildOf(on))).id();

        for arg in cmd.get_arguments() {
            use valence_protocol::packets::play::command_tree_s2c::Parser as ValenceParser;
            let name = arg.get_value_names().unwrap().first().unwrap();
            let name = name.to_ascii_lowercase();
            let node_to_register = hyperion::simulation::command::Command::argument(
                name,
                ValenceParser::String(StringArg::SingleWord),
            );

            on = world.spawn((node_to_register, ChildOf(on))).id();
        }

        let executable = Box::new(GenericExecutableCommand::<Self> { state });

        let tab_complete = |world: &World, completion: &play::RequestCommandCompletions| {
            let compose = world.resource::<Compose>();
            let full_query = &completion.text;
            let id = completion.transaction_id;

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
                        suggested_match: name.into(),
                        tooltip: None,
                    })
                    .collect();

                let start = input_arg.as_ptr() as usize - full_query.as_ptr() as usize;
                let len = input_arg.len();

                let start = i32::try_from(start).unwrap();
                let len = i32::try_from(len).unwrap();

                let packet = CommandSuggestionsS2c {
                    id,
                    start: VarInt(start),
                    length: VarInt(len),
                    matches,
                };

                compose
                    .unicast(&packet, completion.connection_id())
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
                    suggested_match: name.into(),
                    tooltip: None,
                })
                .collect();

            let start = full_query.len();
            let start = i32::try_from(start).unwrap();

            let packet = CommandSuggestionsS2c {
                id,
                start: VarInt(start),
                length: VarInt(0),
                matches,
            };

            compose
                .unicast(&packet, completion.connection_id())
                .unwrap();
        };

        let handler = CommandHandler {
            executable,
            tab_complete,
            has_permissions,
        };

        tracing::info!("registering command {name}");

        let mut registry = world.resource_mut::<CommandRegistry>();
        registry
            .get_mut()
            .unwrap()
            .register(name.as_str().to_string(), handler);
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
    type State = SystemState<Commands<'static, 'static>>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let mut commands = state.get(world);
        let compose = world.resource::<Compose>();
        let ign_map = world.resource::<IgnMap>();
        let Some(&connection_id) = world.entity(caller).get::<ConnectionId>() else {
            error!("permission command failed: caller is missing ConnectionId component");
            return;
        };
        match self {
            Self::Set(cmd) => {
                // Handle setting permissions
                let Some(&entity) = ign_map.get(cmd.player.as_str()) else {
                    let msg = format!("§c{} not found", cmd.player);
                    let chat = hyperion::net::agnostic::chat(msg);
                    compose.unicast(&chat, connection_id).unwrap();
                    return;
                };

                commands.entity(entity).insert(cmd.group);

                let msg = format!(
                    "§b{}§r's group has been set to §e{:?}",
                    cmd.player, cmd.group
                );
                let chat = hyperion::net::agnostic::chat(msg);
                compose.unicast(&chat, connection_id).unwrap();
            }
            Self::Get(cmd) => {
                let Some(&entity) = ign_map.get(cmd.player.as_str()) else {
                    let msg = format!("§c{} not found", cmd.player);
                    let chat = hyperion::net::agnostic::chat(msg);
                    compose.unicast(&chat, connection_id).unwrap();
                    return;
                };

                let Some(group) = world.entity(entity).get::<Group>() else {
                    error!("permission command failed: player is missing Group component");
                    return;
                };

                let msg = format!("§b{}§r's group is §e{:?}", cmd.player, group);
                let chat = hyperion::net::agnostic::chat(msg);
                compose.unicast(&chat, connection_id).unwrap();
            }
        }
    }
}

pub struct ClapCommandPlugin;

impl Plugin for ClapCommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(hyperion_command::CommandPlugin);
        PermissionCommand::register(app.world_mut());
    }
}
