//! The command tree, as the ECS holds it and as 776 sends it.
//!
//! Commands are entities: a node is a [`Command`] component and its children
//! are its flecs children, so a module registers a command by parenting an
//! entity onto [`get_root_command_entity`]. [`get_command_packet`] flattens
//! that into the index-addressed graph
//! `ClientboundCommandsPacket` wants.
//!
//! # Argument types are still spelled in valence's vocabulary
//!
//! `hyperion-clap` builds every argument node and names its parser with
//! valence's 1.20.1 [`Parser`], so that is what [`Command::argument`] takes.
//! [`argument_type`] is the single place that vocabulary meets 776, and it is
//! the piece to delete once callers name a
//! [`hyperion_minecraft_proto::packets::play::player::ArgumentType`] directly.
//! It resolves every id through `minecraft:command_argument_type` rather than
//! writing numbers down, so a protocol bump moves the ids without touching
//! this file. Two names did move between the two versions: 1.20.1's
//! `minecraft:color` is 26.2's `minecraft:team_color`, and `minecraft:time`
//! grew a minimum-duration property that 1.20.1 did not have.

use std::io::Write;

use anyhow::Context as _;
use flecs_ecs::{
    core::{
        Builder, Entity, EntityView, EntityViewGet, IdOperations, QueryAPI, QueryBuilderImpl,
        QueryFlags, World,
    },
    macros::Component,
};
use hyperion_minecraft_proto::{
    Identifier, RegistryId,
    generated::{packet_id::play::clientbound::PacketId, registry},
    packets::play::player::{
        ArgumentType, CommandNode, CommandNodeStub, Commands, StringArgumentKind,
    },
};
use tracing::warn;
pub use valence_protocol::packets::play::command_tree_s2c::Parser;
use valence_protocol::packets::play::command_tree_s2c::StringArg;

use crate::{PacketBundle, net::protocol::Clientbound};

/// What one node of the command tree matches.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeData {
    /// The tree's root, which matches nothing and only has children.
    Root,
    /// A fixed word, e.g. the `fly` in `/fly`.
    Literal {
        /// The word itself.
        name: String,
    },
    /// A parsed value.
    Argument {
        /// Argument name, which is how a suggestion request refers to it.
        name: String,
        /// How the client parses the value.
        parser: Parser,
    },
}

/// One node of the command tree, held on the entity that owns it.
#[derive(Component)]
pub struct Command {
    data: NodeData,
    has_permission: fn(world: &World, caller: Entity) -> bool,
}

pub(crate) static ROOT_COMMAND: once_cell::sync::OnceCell<Entity> =
    once_cell::sync::OnceCell::new();

pub fn get_root_command_entity() -> Entity {
    *ROOT_COMMAND.get().unwrap()
}

/// Relation on a command argument node: `(Suggests, tag)` says the values a
/// client may complete this argument to are the entities carrying `tag`.
///
/// A relation rather than a registry of strings because the candidates already
/// exist as entities, and a second copy of them is a second thing to keep in
/// step. `/kit` suggests whatever carries smash's `Kit` tag at the moment the
/// player presses tab, so a kit added, renamed or removed changes what the
/// client offers with nothing else edited.
#[derive(Component)]
pub struct Suggests;

/// On a tag entity that is the target of [`Suggests`]: how to render one of the
/// entities carrying it as the text a client completes to. Returning `None`
/// leaves that entity out.
///
/// It sits on the tag rather than on the argument node because it describes the
/// thing being suggested, not the argument doing the suggesting. Every argument
/// that suggests kits renders a kit the same way, and only the kit module knows
/// that a kit's name is in its `KitName`.
#[derive(Component)]
pub struct SuggestionLabel(pub fn(EntityView<'_>) -> Option<String>);

/// Completions an argument's own type already knows.
///
/// `hyperion-clap` fills this in from a clap `ValueEnum`'s possible values, so
/// an argument whose type enumerates itself completes without anything being
/// declared anywhere. A node carrying this ignores [`Suggests`].
#[derive(Component)]
pub struct FixedSuggestions(pub Vec<String>);

/// Every value `node` may be completed to right now.
///
/// Answers from [`FixedSuggestions`] when the argument's type knows its own
/// values, and otherwise follows every `(Suggests, tag)` edge and asks each
/// entity carrying `tag` for its [`SuggestionLabel`]. A node with neither has
/// no completions, which is the right answer for a free-form argument such as
/// a chat message.
///
/// The query matches prefabs and disabled entities, because a completion source
/// is game state in whatever form the module that owns it chose: smash's kits
/// are prefabs, and flecs leaves prefabs out of a query unless asked.
#[must_use]
pub fn suggestions(world: &World, node: Entity) -> Vec<String> {
    let view = world.entity_from_id(node);

    if let Some(fixed) = view.try_get::<&FixedSuggestions>(|fixed| fixed.0.clone()) {
        return fixed;
    }

    let mut tags = Vec::new();
    view.each_target(Suggests, |tag| tags.push(tag.id()));

    let mut out = Vec::new();
    for tag in tags {
        let Some(label) = world
            .entity_from_id(tag)
            .try_get::<&SuggestionLabel>(|label| label.0)
        else {
            warn!("a command argument suggests {tag}, which carries no SuggestionLabel");
            continue;
        };

        world
            .query::<()>()
            .with(tag)
            .query_flags(QueryFlags::MatchPrefab | QueryFlags::MatchDisabled)
            .build()
            .each_entity(|entity, ()| {
                if let Some(text) = label(entity) {
                    out.push(text);
                }
            });
    }
    out
}

impl Command {
    pub const ROOT: Self = Self {
        data: NodeData::Root,
        has_permission: |_: _, _: _| true,
    };

    #[must_use]
    pub fn literal(
        name: impl Into<String>,
        has_permission: fn(world: &World, caller: Entity) -> bool,
    ) -> Self {
        Self {
            data: NodeData::Literal { name: name.into() },
            has_permission,
        }
    }

    #[must_use]
    pub fn argument(name: impl Into<String>, parser: Parser) -> Self {
        Self {
            data: NodeData::Argument {
                name: name.into(),
                parser,
            },
            has_permission: |_: _, _: _| true,
        }
    }
}

/// Which suggestion provider an argument node names.
///
/// `minecraft:ask_server` is the only one that can reflect state the client
/// does not have, and every hyperion argument is server state.
const ASK_SERVER: &str = "minecraft:ask_server";

/// The `minecraft:command_argument_type` id of `name`.
fn argument_type_id(name: &str) -> anyhow::Result<RegistryId> {
    let id = registry::COMMAND_ARGUMENT_TYPE
        .id_of(name)
        .with_context(|| format!("no argument type named {name} in this protocol version"))?;
    Ok(RegistryId(i32::try_from(id)?))
}

/// An argument type that writes no properties, by name.
fn empty_argument(name: &str) -> anyhow::Result<ArgumentType<'static>> {
    Ok(ArgumentType::Empty(argument_type_id(name)?))
}

/// A registry-scoped argument type, whose payload is the registry it scopes to.
fn registry_argument<'a>(
    name: &str,
    scope: &'a valence_ident::Ident,
) -> anyhow::Result<ArgumentType<'a>> {
    Ok(ArgumentType::Registry {
        id: argument_type_id(name)?,
        registry: Identifier::new(scope.as_str())?,
    })
}

/// The 776 argument type a 1.20.1 [`Parser`] means.
///
/// # Errors
/// Returns an error when the name has no entry in this version's
/// `minecraft:command_argument_type`, which means valence's vocabulary and the
/// registry have drifted apart and the node cannot be sent at all.
pub fn argument_type(parser: &Parser) -> anyhow::Result<ArgumentType<'_>> {
    Ok(match parser {
        Parser::Bool => empty_argument("brigadier:bool")?,
        Parser::Float { min, max } => ArgumentType::Float {
            min: *min,
            max: *max,
        },
        Parser::Double { min, max } => ArgumentType::Double {
            min: *min,
            max: *max,
        },
        Parser::Integer { min, max } => ArgumentType::Integer {
            min: *min,
            max: *max,
        },
        Parser::Long { min, max } => ArgumentType::Long {
            min: *min,
            max: *max,
        },
        Parser::String(kind) => ArgumentType::String(match kind {
            StringArg::SingleWord => StringArgumentKind::SingleWord,
            StringArg::QuotablePhrase => StringArgumentKind::QuotablePhrase,
            StringArg::GreedyPhrase => StringArgumentKind::GreedyPhrase,
        }),
        Parser::Entity {
            single,
            only_players,
        } => ArgumentType::Entity {
            single: *single,
            players_only: *only_players,
        },
        Parser::ScoreHolder { allow_multiple } => ArgumentType::ScoreHolder {
            multiple: *allow_multiple,
        },
        // 1.20.1's `minecraft:time` carried no properties; 1.20.5 gave it a
        // minimum duration, and zero is the value brigadier's own
        // `TimeArgument.time()` uses.
        Parser::Time => ArgumentType::Time { min: 0 },
        Parser::ResourceOrTag { registry } => {
            registry_argument("minecraft:resource_or_tag", registry)?
        }
        Parser::ResourceOrTagKey { registry } => {
            registry_argument("minecraft:resource_or_tag_key", registry)?
        }
        Parser::Resource { registry } => registry_argument("minecraft:resource", registry)?,
        Parser::ResourceKey { registry } => registry_argument("minecraft:resource_key", registry)?,
        // Everything below is a `SingletonArgumentInfo`, which writes its id
        // and stops.
        Parser::GameProfile => empty_argument("minecraft:game_profile")?,
        Parser::BlockPos => empty_argument("minecraft:block_pos")?,
        Parser::ColumnPos => empty_argument("minecraft:column_pos")?,
        Parser::Vec3 => empty_argument("minecraft:vec3")?,
        Parser::Vec2 => empty_argument("minecraft:vec2")?,
        Parser::BlockState => empty_argument("minecraft:block_state")?,
        Parser::BlockPredicate => empty_argument("minecraft:block_predicate")?,
        Parser::ItemStack => empty_argument("minecraft:item_stack")?,
        Parser::ItemPredicate => empty_argument("minecraft:item_predicate")?,
        // Renamed in 1.20.5: what valence calls `Color` is a team colour.
        Parser::Color => empty_argument("minecraft:team_color")?,
        Parser::Component => empty_argument("minecraft:component")?,
        Parser::Message => empty_argument("minecraft:message")?,
        Parser::NbtCompoundTag => empty_argument("minecraft:nbt_compound_tag")?,
        Parser::NbtTag => empty_argument("minecraft:nbt_tag")?,
        Parser::NbtPath => empty_argument("minecraft:nbt_path")?,
        Parser::Objective => empty_argument("minecraft:objective")?,
        Parser::ObjectiveCriteria => empty_argument("minecraft:objective_criteria")?,
        Parser::Operation => empty_argument("minecraft:operation")?,
        Parser::Particle => empty_argument("minecraft:particle")?,
        Parser::Angle => empty_argument("minecraft:angle")?,
        Parser::Rotation => empty_argument("minecraft:rotation")?,
        Parser::ScoreboardSlot => empty_argument("minecraft:scoreboard_slot")?,
        Parser::Swizzle => empty_argument("minecraft:swizzle")?,
        Parser::Team => empty_argument("minecraft:team")?,
        Parser::ItemSlot => empty_argument("minecraft:item_slot")?,
        Parser::ResourceLocation => empty_argument("minecraft:resource_location")?,
        Parser::Function => empty_argument("minecraft:function")?,
        Parser::EntityAnchor => empty_argument("minecraft:entity_anchor")?,
        Parser::IntRange => empty_argument("minecraft:int_range")?,
        Parser::FloatRange => empty_argument("minecraft:float_range")?,
        Parser::Dimension => empty_argument("minecraft:dimension")?,
        Parser::GameMode => empty_argument("minecraft:gamemode")?,
        Parser::TemplateMirror => empty_argument("minecraft:template_mirror")?,
        Parser::TemplateRotation => empty_argument("minecraft:template_rotation")?,
        Parser::Uuid => empty_argument("minecraft:uuid")?,
    })
}

/// One flattened node: what it matches and which indices follow it.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    /// What the node matches.
    pub data: NodeData,
    /// Indices of the nodes that may follow it.
    pub children: Vec<i32>,
    /// Whether a command may end here.
    pub executable: bool,
}

/// The command tree, flattened and ready to send.
///
/// The root is index zero, which is what [`get_command_packet`] pushes first.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandTree {
    /// Every node, root first.
    pub nodes: Vec<TreeNode>,
}

impl PacketBundle for &CommandTree {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let stub = match &node.data {
                NodeData::Root => CommandNodeStub::Root,
                NodeData::Literal { name } => CommandNodeStub::Literal { name },
                NodeData::Argument { name, parser } => CommandNodeStub::Argument {
                    name,
                    parser: argument_type(parser)?,
                    suggestions: Some(Identifier::new(ASK_SERVER)?),
                },
            };
            nodes.push(CommandNode {
                children: node.children.clone(),
                redirect: None,
                stub,
                executable: node.executable,
                // hyperion filters the tree per player instead of greying
                // nodes out, so a node a player can see is one they may run.
                restricted: false,
            });
        }

        let body = Commands {
            nodes,
            root_index: 0,
        };
        Clientbound::new(PacketId::Commands.to_raw(), &body).encode_including_ids(w)
    }
}

/// Deepest the walk will follow a parent chain before giving up.
const MAX_DEPTH: usize = 64;

/// Flatten the tree under `root` into the graph the client rebuilds.
///
/// When `player_opt` is given, a node whose permission predicate rejects that
/// player is dropped along with everything under it, so the client never learns
/// a command exists that it may not run.
pub fn get_command_packet(world: &World, root: Entity, player_opt: Option<Entity>) -> CommandTree {
    struct StackElement {
        depth: usize,
        ptr: usize,
        entity: Entity,
    }

    let mut nodes = Vec::new();

    let mut stack = vec![StackElement {
        depth: 0,
        ptr: 0,
        entity: root,
    }];

    nodes.push(TreeNode {
        data: NodeData::Root,
        children: Vec::new(),
        executable: false,
    });

    while let Some(StackElement {
        depth,
        entity,
        ptr: parent_ptr,
    }) = stack.pop()
    {
        if depth >= MAX_DEPTH {
            warn!("command tree depth exceeded. Exiting early. Circular reference?");
            break;
        }

        world.entity_from_id(entity).each_child(|child| {
            child.get::<&Command>(|command| {
                if let Some(player) = player_opt
                    && !(command.has_permission)(world, player)
                {
                    return;
                }

                let ptr = nodes.len();

                nodes.push(TreeNode {
                    data: command.data.clone(),
                    children: Vec::new(),
                    executable: true,
                });

                let node = &mut nodes[parent_ptr];
                node.children.push(i32::try_from(ptr).unwrap());

                stack.push(StackElement {
                    depth: depth + 1,
                    ptr,
                    entity: child.id(),
                });
            });
        });
    }

    CommandTree { nodes }
}

#[cfg(test)]
mod tests {
    use flecs_ecs::core::ComponentId;
    use hyperion_minecraft_proto::{Decode, Reader};

    use super::*;

    /// Stands in for smash's `Kit`: a tag whose bearers are what an argument
    /// completes to.
    #[derive(Component)]
    struct Choice;

    /// Stands in for `KitName`.
    #[derive(Component)]
    struct Label(&'static str);

    /// A world with the completion vocabulary registered, a `Choice` tag that
    /// knows how to render itself, and one argument node pointed at it.
    fn completion_world() -> (World, Entity) {
        let world = World::new();
        world.component::<Command>();
        world.component::<Suggests>();
        world.component::<SuggestionLabel>();
        world.component::<FixedSuggestions>();
        world.component::<Label>();
        world.component::<Choice>().set(SuggestionLabel(|entity| {
            entity.try_get::<&Label>(|label| label.0.to_owned())
        }));

        let node = world.entity().set(Command::argument(
            "choice",
            Parser::String(StringArg::GreedyPhrase),
        ));
        node.add((Suggests, Choice::id()));

        let id = node.id();
        (world, id)
    }

    #[test]
    fn an_argument_with_no_source_offers_nothing() {
        let world = World::new();
        world.component::<Command>();
        world.component::<Suggests>();
        world.component::<SuggestionLabel>();
        world.component::<FixedSuggestions>();

        let node = world.entity().set(Command::argument(
            "message",
            Parser::String(StringArg::GreedyPhrase),
        ));

        assert!(suggestions(&world, node.id()).is_empty());
    }

    #[test]
    fn suggestions_are_whatever_carries_the_tag_right_now() {
        let (world, node) = completion_world();

        // Prefabs, which is the shape smash's kits take, and which flecs leaves
        // out of a query unless it is asked for them.
        world
            .prefab_named("golem")
            .add(Choice::id())
            .set(Label("Iron Golem"));
        let skeleton = world
            .prefab_named("skeleton")
            .add(Choice::id())
            .set(Label("Skeleton"));

        let mut offered = suggestions(&world, node);
        offered.sort();
        assert_eq!(offered, vec![
            "Iron Golem".to_owned(),
            "Skeleton".to_owned()
        ]);

        // The point of the relation: the completions follow the world rather
        // than a list somebody has to remember to edit.
        skeleton.destruct();
        assert_eq!(suggestions(&world, node), vec!["Iron Golem".to_owned()]);

        world
            .prefab_named("wolf")
            .add(Choice::id())
            .set(Label("Wolf"));
        let mut offered = suggestions(&world, node);
        offered.sort();
        assert_eq!(offered, vec!["Iron Golem".to_owned(), "Wolf".to_owned()]);
    }

    #[test]
    fn a_bearer_with_no_label_is_left_out_rather_than_offered_blank() {
        let (world, node) = completion_world();

        world.entity().add(Choice::id()).set(Label("Named"));
        world.entity().add(Choice::id());

        assert_eq!(suggestions(&world, node), vec!["Named".to_owned()]);
    }

    #[test]
    fn a_type_that_knows_its_own_values_answers_without_the_world() {
        let (world, node) = completion_world();

        world
            .prefab_named("golem")
            .add(Choice::id())
            .set(Label("Iron Golem"));

        // What `hyperion-clap` writes for a clap `ValueEnum`. It wins over the
        // relation, because an argument whose type enumerates itself cannot
        // accept anything else.
        world.entity_from_id(node).set(FixedSuggestions(vec![
            "survival".to_owned(),
            "creative".to_owned(),
        ]));

        assert_eq!(suggestions(&world, node), vec![
            "survival".to_owned(),
            "creative".to_owned()
        ]);
    }

    /// The bytes a tree encodes to, id byte and all.
    fn encode(tree: &CommandTree) -> Vec<u8> {
        let mut bytes = Vec::new();
        tree.encode_including_ids(&mut bytes).expect("encode");
        bytes
    }

    #[test]
    fn test_empty_command_tree() {
        let world = World::new();
        world.component::<Command>();
        let root = world.entity();

        let tree = get_command_packet(&world, root.id(), None);

        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].data, NodeData::Root);
        assert!(tree.nodes[0].children.is_empty());
    }

    #[test]
    fn test_single_command() {
        let world = World::new();
        world.component::<Command>();
        let root = world.entity();

        world
            .entity()
            .set(Command {
                data: NodeData::Literal {
                    name: "test".to_owned(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(root);

        let tree = get_command_packet(&world, root.id(), None);

        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.nodes[1].data, NodeData::Literal {
            name: "test".to_owned(),
        });
    }

    #[test]
    fn test_nested_commands() {
        let world = World::new();

        world.component::<Command>();

        let root = world.entity();

        let parent = world
            .entity()
            .set(Command {
                data: NodeData::Literal {
                    name: "parent".to_owned(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(root);

        let _child = world
            .entity()
            .set(Command {
                data: NodeData::Literal {
                    name: "child".to_owned(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(parent);

        let tree = get_command_packet(&world, root.id(), None);

        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.nodes[1].children, vec![2]);
        assert_eq!(tree.nodes[1].data, NodeData::Literal {
            name: "parent".to_owned(),
        });
        assert_eq!(tree.nodes[2].data, NodeData::Literal {
            name: "child".to_owned(),
        });
    }

    #[test]
    fn test_max_depth() {
        let world = World::new();
        world.component::<Command>();

        let root = world.entity();

        let mut parent = root;
        for i in 0..=MAX_DEPTH {
            let child = world
                .entity()
                .set(Command {
                    data: NodeData::Literal {
                        name: format!("command_{i}"),
                    },
                    has_permission: |_: _, _: _| true,
                })
                .child_of(parent);
            parent = child;
        }

        let tree = get_command_packet(&world, root.id(), None);

        assert_eq!(tree.nodes.len(), MAX_DEPTH + 1);
    }

    /// The whole point of the port: the tree goes out as play id 0x10 in 776's
    /// numbering, and an argument node names `brigadier:string` by the id that
    /// registry gives it rather than by 1.20.1's.
    #[test]
    fn encodes_as_776() {
        let tree = CommandTree {
            nodes: vec![
                TreeNode {
                    data: NodeData::Root,
                    children: vec![1],
                    executable: false,
                },
                TreeNode {
                    data: NodeData::Literal {
                        name: "team".to_owned(),
                    },
                    children: vec![2],
                    executable: false,
                },
                TreeNode {
                    data: NodeData::Argument {
                        name: "name".to_owned(),
                        parser: Parser::String(StringArg::SingleWord),
                    },
                    children: Vec::new(),
                    executable: true,
                },
            ],
        };

        let bytes = encode(&tree);
        assert_eq!(bytes[0], 0x10, "clientbound play id of minecraft:commands");

        let mut reader = Reader::new(&bytes[1..]);
        let decoded = Commands::decode(&mut reader).expect("decode");
        reader.finish().expect("packet body fully consumed");

        assert_eq!(decoded.root_index, 0);
        let CommandNodeStub::Argument { parser, .. } = decoded.nodes[2].stub else {
            panic!("third node should be the argument");
        };
        assert_eq!(
            parser.to_id().expect("argument type id"),
            i32::try_from(
                registry::COMMAND_ARGUMENT_TYPE
                    .id_of("brigadier:string")
                    .expect("brigadier:string is registered")
            )
            .expect("id fits an i32")
        );
        assert!(decoded.nodes[2].executable);
    }
}
