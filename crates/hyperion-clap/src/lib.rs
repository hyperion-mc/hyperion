//! Minecraft commands derived from clap.
//!
//! A command reaches a player through two protocol mechanisms, and this crate
//! feeds both of them off the same clap definition:
//!
//! * the **command graph**, sent once when a player joins, which is what makes
//!   a slash list the server's commands and tells the client how to parse each
//!   argument. [`MinecraftCommand::register`] builds it out of the derive:
//!   subcommands become literal nodes, positionals become argument nodes, and
//!   an argument taking more than one value becomes a greedy string so a kit
//!   name with a space in it is one value rather than two.
//! * **command suggestions**, the request and response pair the client sends
//!   when a player presses tab. [`completion::locate`] decides which node the
//!   cursor is on, and [`hyperion::simulation::command::suggestions`] asks that
//!   node what it may be completed to.
//!
//! # Declaring where an argument's completions come from
//!
//! An argument whose clap type already enumerates itself needs no declaration:
//! a `ValueEnum` positional carries its possible values into the graph at
//! registration. An argument completed from live game state names the flecs tag
//! its candidates carry:
//!
//! ```ignore
//! KitCommand::register(registry, world).completes("name", Kit::id());
//! ```
//!
//! and the module that owns that tag says once how to render one of them:
//!
//! ```ignore
//! world
//!     .component::<Kit>()
//!     .set(SuggestionLabel(|kit| kit.try_get::<&KitName>(|name| name.0.to_owned())));
//! ```
//!
//! Nothing between those two lines is a list of kits. The completions are a
//! query over whatever carries the tag when the player presses tab.
//!
//! `hyperion::simulation::Player` arrives with its label already set, so an
//! argument naming a player is one line and no new vocabulary:
//!
//! ```ignore
//! PermissionCommand::register(registry, world).completes("player", Player::id());
//! ```
//!
//! # A permission is per subcommand, and it is checked twice
//!
//! Both mechanisms above are gated, and they are gated separately because they
//! fail differently. The command tree leaves out any node the player may not
//! use, so a `Normal` player is never offered `/perms set` -- getting that
//! wrong costs a misleading autocomplete. [`MinecraftCommand::register`]'s
//! execute path checks the *parsed* invocation before running it -- getting
//! that wrong is a privilege escalation, and it was one: the derive could only
//! carry one group for a whole `clap` enum, so `/perms` was `Normal` because
//! `/perms get` is, and every player could run `/perms set` (ENG-10871).
//!
//! Which group each subcommand needs is declared on it. See
//! [`hyperion_clap_macros`] for the shapes the derive accepts and the ones it
//! refuses. Nobody starts out holding a group that a command hands out, so the
//! first administrator is named in configuration rather than appointed:
//! [`hyperion_permission::seed`].

pub mod completion;

use std::sync::Arc;

use clap::{Arg as ClapArg, Parser, ValueEnum, ValueHint, error::ErrorKind};
use flecs_ecs::{
    core::{
        ComponentId, Entity, EntityView, EntityViewGet, IdOperations, IntoEntity, World, WorldGet,
        WorldProvider, id,
    },
    prelude::{Component, Module},
};
use hyperion::{
    hyperion_minecraft_proto::{
        generated::packet_id::play::clientbound::PacketId,
        packets::play::clientbound::{CommandSuggestions, command_suggestions},
    },
    net::{Compose, ConnectionId, DataBundle, agnostic, protocol::Clientbound},
    simulation::{
        IgnMap, Player,
        command::{
            FixedSuggestions, HasPermission, Suggests, get_root_command_entity, suggestions,
        },
        handlers::PacketSwitchQuery,
    },
    storage::CommandCompletionRequest,
};
pub use hyperion_clap_macros::CommandPermission;
pub use hyperion_command;
use hyperion_command::{CommandHandler, CommandRegistry};
use hyperion_permission::Group;
use valence_protocol::packets::play::command_tree_s2c::{Parser as ValenceParser, StringArg};

use crate::completion::Step;

/// Whether a clap argument takes more than one value, which makes it one
/// greedy Brigadier string rather than a run of single words.
fn is_greedy(arg: &ClapArg) -> bool {
    arg.get_num_args()
        .is_some_and(|range| range.max_values() > 1)
}

/// The gate for one node of `C`'s tree: the command's own node when
/// `subcommand` is `None`, and that subcommand's otherwise.
fn gate_for<C: MinecraftCommand>(subcommand: Option<String>) -> HasPermission {
    Arc::new(move |world: &World, caller: Entity| {
        caller.entity_view(world).get::<&Group>(|group| {
            C::subcommand_has_required_permission(subcommand.as_deref(), *group)
        })
    })
}

/// Build the graph nodes under `parent` for one clap command, and return the
/// shape the completion walk reads.
///
/// Positionals chain one after another because that is the order they are
/// typed in; subcommands hang off the end of that chain as literals, which for
/// every command here means directly off the command's own node.
///
/// `under` is the subcommand this subtree hangs beneath, which is what decides
/// the gate on every node in it. A subcommand's own children are gated as it
/// is: nothing below `/perms set` can be for a wider audience than `set`.
fn build<C: MinecraftCommand>(
    world: &World,
    parent: Entity,
    cmd: &clap::Command,
    under: Option<&str>,
) -> Vec<Step<Entity>> {
    let mut on = parent;
    let mut created = Vec::new();

    for arg in cmd.get_positionals() {
        let display = arg
            .get_value_names()
            .and_then(|names| names.first())
            .map_or_else(
                || arg.get_id().to_string(),
                |name| name.to_ascii_lowercase(),
            );

        let kind = if is_greedy(arg) {
            StringArg::GreedyPhrase
        } else {
            StringArg::SingleWord
        };
        let node = world
            .entity()
            .set(hyperion::simulation::command::Command::argument(
                display,
                ValenceParser::String(kind),
            ))
            .child_of(on);

        // A clap `ValueEnum` has already written down every value it accepts,
        // so an argument built from one completes with nothing else declared.
        let possible: Vec<String> = arg
            .get_possible_values()
            .iter()
            .map(|value| value.get_name().to_owned())
            .collect();
        if !possible.is_empty() {
            node.set(FixedSuggestions(possible));
        }

        created.push((arg, node.id()));
        on = node.id();
    }

    let mut steps: Vec<Step<Entity>> = cmd
        .get_subcommands()
        .map(|sub| {
            let key = under.unwrap_or_else(|| sub.get_name());
            let literal = world
                .entity()
                .set(hyperion::simulation::command::Command::literal(
                    sub.get_name().to_owned(),
                    gate_for::<C>(Some(key.to_owned())),
                ))
                .child_of(on);
            Step {
                literal: Some(sub.get_name().to_owned()),
                id: sub.get_name().to_owned(),
                node: literal.id(),
                greedy: false,
                next: build::<C>(world, literal.id(), sub, Some(key)),
            }
        })
        .collect();

    // Folded from the back so each positional owns what follows it.
    for (arg, node) in created.into_iter().rev() {
        steps = vec![Step {
            literal: None,
            id: arg.get_id().to_string(),
            node,
            greedy: is_greedy(arg),
            next: steps,
        }];
    }

    steps
}

/// Collect every value step in a shape, so a declaration can name one.
fn value_steps(steps: &[Step<Entity>], out: &mut Vec<(String, Entity)>) {
    for step in steps {
        if step.literal.is_none() {
            out.push((step.id.clone(), step.node));
        }
        value_steps(&step.next, out);
    }
}

/// A registered command, for declaring where its arguments complete from.
///
/// Returned by [`MinecraftCommand::register`]. Ignoring it is fine: a command
/// whose arguments all know their own values, or which has none, needs nothing
/// further.
pub struct Registered<'w> {
    world: &'w World,
    /// Every value step, by the clap id of the field that produced it.
    values: Vec<(String, Entity)>,
}

impl Registered<'_> {
    /// Complete the argument named `arg` from every entity carrying `tag`.
    ///
    /// `arg` is the clap id, which for a derived command is the struct field's
    /// name. `tag` is a flecs component id, and the candidates are whatever
    /// carries it when the player presses tab: this is an edge in the world,
    /// not a snapshot of one.
    ///
    /// The tag needs a [`SuggestionLabel`] saying how to render one of its
    /// entities, which the module owning the tag sets once.
    ///
    /// Every argument by that name, not the first: one clap id can reach more
    /// than one node, because `/perms set <player>` and `/perms get <player>`
    /// are two nodes of the same command and a player means the same thing in
    /// both. Wiring only the first is the kind of half-working completion that
    /// looks like a protocol bug from the client.
    ///
    /// # Panics
    /// When the command has no argument by that name. A completion that
    /// silently never fires is the failure this whole mechanism exists to
    /// avoid, so a typo here stops the server at startup rather than at the
    /// moment somebody presses tab.
    ///
    /// [`SuggestionLabel`]: hyperion::simulation::command::SuggestionLabel
    pub fn completes(&self, arg: &str, tag: impl IntoEntity) -> &Self {
        let tag = tag.into_entity(self.world);

        let mut wired = 0;
        for &(_, node) in self.values.iter().filter(|(id, _)| id == arg) {
            self.world.entity_from_id(node).add((Suggests, tag));
            wired += 1;
        }

        if wired == 0 {
            let known: Vec<&str> = self.values.iter().map(|(id, _)| id.as_str()).collect();
            panic!("no command argument named {arg}; this command has {known:?}");
        }

        self
    }
}

pub trait MinecraftCommand: Parser + CommandPermission + 'static {
    fn execute(self, system: EntityView<'_>, caller: Entity);

    fn pre_register(_world: &World) {}

    /// # Panics
    /// When the command declares a group for some subcommand `clap` does not
    /// have, or has a subcommand it declares no group for. Both mean a
    /// subcommand whose gate is not the one anybody wrote down, and the server
    /// refusing to start is cheaper than finding that out from a player.
    fn register<'w>(registry: &mut CommandRegistry, world: &'w World) -> Registered<'w> {
        Self::pre_register(world);

        let cmd = Self::command();
        let name = cmd.get_name().to_owned();

        // The derive files each subcommand's group under the name it works out
        // that `clap` will use, by kebab-casing the variant. This is where that
        // is checked against the name `clap` actually reports, because the
        // failure it prevents is silent: a group filed under a name no
        // subcommand has leaves that subcommand falling back, and falling back
        // to the weakest group is exactly ENG-10871.
        let declared = Self::declared_subcommands();
        if !declared.is_empty() {
            let actual: Vec<&str> = cmd.get_subcommands().map(clap::Command::get_name).collect();
            let undeclared: Vec<&&str> = actual
                .iter()
                .filter(|name| !declared.contains(name))
                .collect();
            assert!(
                undeclared.is_empty(),
                "/{name} has subcommand(s) {undeclared:?} that carry no permission group. The \
                 derive declared groups for {declared:?} and clap named {actual:?}; the two have \
                 to agree or those subcommands are gated by nothing anybody wrote."
            );
        }

        let has_permissions = |world: &World, caller: Entity| {
            caller
                .entity_view(world)
                .get::<&Group>(|group| Self::has_required_permission(*group))
        };

        let node_to_register =
            hyperion::simulation::command::Command::literal(name.clone(), gate_for::<Self>(None));

        let root = world
            .entity()
            .set(node_to_register)
            .child_of(get_root_command_entity());

        let shape = build::<Self>(world, root.id(), &cmd, None);

        let mut values = Vec::new();
        value_steps(&shape, &mut values);

        let on_execute = |input: &str, system: EntityView<'_>, caller: Entity| {
            let input = input.split_whitespace();
            let world = system.world();

            match Self::try_parse_from(input) {
                Ok(elem) => {
                    // The parsed invocation, not the command. `/perms get` and
                    // `/perms set` are one command and two permissions, and
                    // asking the command was ENG-10871.
                    if world.get::<&Compose>(|compose| {
                        caller.entity_view(world).get::<(&ConnectionId, &Group)>(
                            |(stream, group)| {
                                if elem.variant_has_required_permission(*group) {
                                    true
                                } else {
                                    let chat = agnostic::chat(
                                        "§cYou do not have permission to use this command!",
                                    );

                                    let mut bundle = DataBundle::new(compose);
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
                                compose.unicast(&msg, *stream).unwrap();
                            });
                    });

                    tracing::warn!("could not parse command {e}");
                }
            }
        };

        let on_tab_complete = Box::new(
            move |packet_switch_query: &mut PacketSwitchQuery<'_>,
                  completion: &CommandCompletionRequest| {
                let text = completion.query.as_str();

                // Every request is answered, including with nothing. A client
                // that pressed tab is blocked on this reply: `ask_server` hands
                // the client's own suggestion machinery a future that only this
                // packet completes, so dropping the request stops the player
                // seeing even the suggestions the client worked out for itself.
                let (span, matched) = match completion::locate(&shape, text) {
                    Some(cursor) => {
                        let prefix = cursor.prefix.to_lowercase();
                        let matched: Vec<String> =
                            suggestions(packet_switch_query.world, cursor.node)
                                .into_iter()
                                .filter(|value| value.to_lowercase().starts_with(&prefix))
                                .collect();
                        ((cursor.start, cursor.prefix.len()), matched)
                    }
                    // The cursor is on the command's own name or past the last
                    // argument. Nothing to offer, and an empty span at the end
                    // of the line is the range vanilla replaces with nothing.
                    None => ((text.len(), 0), Vec::new()),
                };

                let (Ok(start), Ok(length)) = (i32::try_from(span.0), i32::try_from(span.1)) else {
                    tracing::warn!("a command longer than 2GB is not worth completing");
                    return;
                };

                let packet = CommandSuggestions {
                    id: completion.id,
                    start,
                    length,
                    suggestions: matched
                        .iter()
                        .map(|text| command_suggestions::Entry {
                            text,
                            tooltip: None,
                        })
                        .collect(),
                };

                if let Err(error) = packet_switch_query.compose.unicast(
                    Clientbound::new(PacketId::CommandSuggestions.to_raw(), &packet),
                    packet_switch_query.io_ref,
                ) {
                    tracing::warn!("dropping a command suggestion response: {error}");
                }
            },
        );

        let handler = CommandHandler {
            on_execute,
            on_tab_complete,
            has_permissions,
        };

        tracing::info!("registering command {name}");

        registry.register(name, handler);

        Registered { world, values }
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

/// Who a command, and each of its subcommands, is for.
///
/// Derived; see [`hyperion_clap_macros`] for how a group is declared. The three
/// questions are separate because they have three different answers, and
/// answering the third with the first is the bug this shape exists to prevent:
/// `/perms` as a command is for everybody, and `/perms set` is not.
pub trait CommandPermission {
    /// Whether `user_group` may use this command at all, which is true when it
    /// may use at least one of the command's subcommands.
    ///
    /// This is a visibility question, not a security one. It decides whether
    /// the command's own node reaches a player's tree and whether
    /// [`hyperion_command::CommandRegistry::get_permitted`] lists it.
    fn has_required_permission(user_group: hyperion_permission::Group) -> bool;

    /// Whether `user_group` may run *this* parsed invocation.
    ///
    /// The security boundary. Every other method on this trait can be wrong
    /// and cost a player a misleading autocomplete; this one being wrong is a
    /// privilege escalation.
    fn variant_has_required_permission(&self, user_group: hyperion_permission::Group) -> bool;

    /// Whether `user_group` may see `subcommand`'s literal in their command
    /// tree. `None` asks about the command's own node.
    fn subcommand_has_required_permission(
        subcommand: Option<&str>,
        user_group: hyperion_permission::Group,
    ) -> bool;

    /// The subcommand names this command declares a group for, empty when one
    /// group covers the whole command.
    ///
    /// [`MinecraftCommand::register`] checks this against the names `clap`
    /// reports and refuses to start the server when they disagree.
    fn declared_subcommands() -> &'static [&'static str];
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

/// `/perms`: read a player's group, or write one.
///
/// The two halves are declared separately and deliberately. Reading a group is
/// a `Normal` thing to do -- it is how a player finds out what they may run --
/// and writing one is the act that hands out every other `Admin` command on the
/// server, permanently, because `PermissionStorage` is keyed on the profile id
/// and survives a restart.
///
/// There is no type-level `#[command_permission]` here on purpose. With one,
/// adding a third subcommand would silently inherit it; without one, the derive
/// refuses to compile until the new subcommand says who it is for.
///
/// The way to appoint the first administrator is not this command, which would
/// be circular. It is [`hyperion_permission::seed`].
#[derive(Parser, CommandPermission, Debug)]
#[command(name = "perms")]
pub enum PermissionCommand {
    #[command_permission(group = "Admin")]
    Set(SetCommand),
    #[command_permission(group = "Normal")]
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
                                compose.unicast(&chat, *stream).unwrap();
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
                                compose.unicast(&chat, *stream).unwrap();
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
                                compose.unicast(&chat, *stream).unwrap();
                            });
                        });
                        return;
                    };

                    entity.entity_view(world).get::<&Group>(|group| {
                        caller.entity_view(world).get::<&ConnectionId>(|stream| {
                            let msg = format!("§b{}§r's group is §e{:?}", cmd.player, group);
                            let chat = hyperion::net::agnostic::chat(msg);
                            world.get::<&Compose>(|compose| {
                                compose.unicast(&chat, *stream).unwrap();
                            });
                        });
                    });
                }
            }
        });
    }
}

/// `/serverload`: show or hide the host's own CPU and memory.
///
/// Admin, and opt in. Fifteen players in a lobby staring at server telemetry
/// is a strange default, and three stacked bars is most of the strip; this way
/// the operator who wants the numbers has them and nobody else pays for it.
///
/// Everything it does is one relationship. `egress::server_load::toggle` adds
/// or removes `(ShownTo, caller)` on the two bars, and the `Add` packets, the
/// `Remove` packets and the diffing in between are `egress::boss_bar`'s.
#[derive(Parser, CommandPermission, Debug)]
#[command(name = "serverload")]
#[command_permission(group = "Admin")]
pub struct ServerLoadCommand;

impl MinecraftCommand for ServerLoadCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let showing = hyperion::egress::server_load::toggle(world, caller);
        let message = if showing {
            "§aServer load bars on."
        } else {
            "§7Server load bars off."
        };
        let chat = hyperion::net::agnostic::chat(message);
        world.get::<&Compose>(|compose| {
            caller.entity_view(world).get::<&ConnectionId>(|stream| {
                if let Err(error) = compose.unicast(&chat, *stream) {
                    tracing::warn!("dropping a /serverload reply: {error}");
                }
            });
        });
    }
}

impl Module for ClapCommandModule {
    fn module(world: &World) {
        world.import::<hyperion_command::CommandModule>();

        world.get::<&mut CommandRegistry>(|registry| {
            // Both spellings of the player argument, `set` and `get`, complete
            // from whoever is connected. The group argument declares nothing:
            // it is a clap `ValueEnum`, so registration already carried its
            // four values into the graph.
            PermissionCommand::register(registry, world).completes("player", Player::id());
            ServerLoadCommand::register(registry, world);
        });
    }
}

#[cfg(test)]
mod permission_gates {
    use clap::{CommandFactory, Parser};
    use hyperion_permission::Group;

    use super::{CommandPermission, PermissionCommand, ServerLoadCommand};

    fn parse(line: &str) -> PermissionCommand {
        PermissionCommand::try_parse_from(line.split_whitespace()).unwrap()
    }

    /// ENG-10871. Before the derive could carry a group per variant this
    /// assertion read `assert!(...)` and passed, because `Set` and `Get` were
    /// one gate and `Get` set it.
    #[test]
    fn a_normal_player_cannot_write_a_group() {
        let set = parse("perms set Andrew admin");
        assert!(!set.variant_has_required_permission(Group::Normal));
        assert!(!set.variant_has_required_permission(Group::Moderator));
        assert!(set.variant_has_required_permission(Group::Admin));
    }

    /// And the half that was always meant to be open stays open, which is the
    /// reason this could not be fixed by moving the whole command to `Admin`.
    #[test]
    fn a_normal_player_can_still_read_one() {
        assert!(parse("perms get Andrew").variant_has_required_permission(Group::Normal));
    }

    /// A banned player is not a weak player: they are refused both halves.
    #[test]
    fn a_banned_player_gets_neither() {
        assert!(!parse("perms get Andrew").variant_has_required_permission(Group::Banned));
        assert!(!parse("perms set Andrew admin").variant_has_required_permission(Group::Banned));
    }

    /// `/perms` itself stays visible to everybody, because `get` is: the
    /// command-level question is "may you use any of this", and answering it
    /// with the strongest variant would hide a command players may run.
    #[test]
    fn the_command_is_visible_to_whoever_may_use_any_of_it() {
        assert!(PermissionCommand::has_required_permission(Group::Normal));
        assert!(!PermissionCommand::has_required_permission(Group::Banned));
    }

    /// The tree a `Normal` player is sent has `/perms get` in it and not
    /// `/perms set`. A unit test on the execute path would pass while the
    /// client was still being offered the command.
    #[test]
    fn the_tree_offers_only_the_half_the_player_may_use() {
        assert!(PermissionCommand::subcommand_has_required_permission(
            Some("get"),
            Group::Normal
        ));
        assert!(!PermissionCommand::subcommand_has_required_permission(
            Some("set"),
            Group::Normal
        ));
        assert!(PermissionCommand::subcommand_has_required_permission(
            Some("set"),
            Group::Admin
        ));
    }

    /// The same check `register` makes at startup, made here too so it fails in
    /// `nix run .#test` rather than on a server nobody has booted yet. The
    /// derive kebab-cases variant names to guess what `clap` will call them,
    /// and this is what says the guess was right.
    #[test]
    fn every_subcommand_clap_has_carries_a_declared_group() {
        let command = PermissionCommand::command();
        let actual: Vec<&str> = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        let declared = PermissionCommand::declared_subcommands();

        assert!(!actual.is_empty(), "perms has subcommands");
        for name in &actual {
            assert!(
                declared.contains(name),
                "clap calls a subcommand {name:?} and the derive declared {declared:?}"
            );
        }
    }

    /// A command with one group covering it declares no subcommands, so the
    /// startup check does not apply to it and its execution gate is its own.
    #[test]
    fn a_single_group_command_is_unchanged() {
        assert!(ServerLoadCommand::declared_subcommands().is_empty());
        assert!(ServerLoadCommand.variant_has_required_permission(Group::Admin));
        assert!(!ServerLoadCommand.variant_has_required_permission(Group::Moderator));
        assert!(!ServerLoadCommand.variant_has_required_permission(Group::Normal));
    }
}
