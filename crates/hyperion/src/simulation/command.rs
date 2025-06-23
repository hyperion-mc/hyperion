use bevy::prelude::*;
use derive_more::Deref;
use tracing::{error, warn};
use valence_bytes::Utf8Bytes;
pub use valence_protocol::packets::play::command_tree_s2c::Parser;
use valence_protocol::{
    VarInt,
    packets::play::command_tree_s2c::{Node, NodeData, Suggestion},
};

#[derive(Component)]
pub struct Command {
    data: NodeData,
    has_permission: fn(world: &World, caller: Entity) -> bool,
}

#[derive(Resource, Deref)]
pub struct RootCommand(Entity);

impl Command {
    pub const ROOT: Self = Self {
        data: NodeData::Root,
        has_permission: |_: _, _: _| true,
    };

    #[must_use]
    pub fn literal(
        name: impl Into<Utf8Bytes>,
        has_permission: fn(world: &World, caller: Entity) -> bool,
    ) -> Self {
        let name = name.into();
        Self {
            data: NodeData::Literal { name },
            has_permission,
        }
    }

    #[must_use]
    pub fn argument(name: impl Into<Utf8Bytes>, parser: Parser) -> Self {
        let name = name.into();
        Self {
            data: NodeData::Argument {
                name,
                parser,
                suggestion: Some(Suggestion::AskServer),
            },
            has_permission: |_: _, _: _| true,
        }
    }
}

// we want a get command packet

const MAX_DEPTH: usize = 64;

pub fn get_command_packet(
    world: &World,
    player_opt: Option<Entity>,
) -> valence_protocol::packets::play::CommandTreeS2c {
    struct StackElement {
        depth: usize,
        ptr: usize,
        entity: Entity,
    }

    let root = world.resource::<RootCommand>();

    let mut commands = Vec::new();

    let mut stack = vec![StackElement {
        depth: 0,
        ptr: 0,
        entity: **root,
    }];

    commands.push(Node {
        data: NodeData::Root,
        executable: false,
        children: vec![],
        redirect_node: None,
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

        let Some(children) = world.entity(entity).get::<Children>() else {
            continue;
        };

        for &child in children {
            let Some(command) = world.entity(child).get::<Command>() else {
                error!("child is missing Command component");
                continue;
            };

            if let Some(player) = player_opt
                && !(command.has_permission)(world, player)
            {
                continue;
            }

            let ptr = commands.len();

            commands.push(Node {
                data: command.data.clone(),
                executable: true,
                children: Vec::new(),
                redirect_node: None,
            });

            let node = &mut commands[parent_ptr];
            node.children.push(i32::try_from(ptr).unwrap().into());

            stack.push(StackElement {
                depth: depth + 1,
                ptr,
                entity: child,
            });
        }
    }

    valence_protocol::packets::play::CommandTreeS2c {
        commands,
        root_index: VarInt(0),
    }
}

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        let root_command = app.world_mut().spawn(Command::ROOT).id();
        app.insert_resource(RootCommand(root_command));
    }
}

#[cfg(test)]
mod tests {
    use flecs_ecs::prelude::*;

    use super::*;

    #[test]
    fn test_empty_command_tree() {
        let world = World::new();
        world.component::<Command>();
        let root = world.entity();

        let packet = get_command_packet(&world, root.id(), None);

        assert_eq!(packet.commands.len(), 1);
        assert_eq!(packet.root_index, VarInt(0));
        assert_eq!(packet.commands[0].data, NodeData::Root);
        assert!(packet.commands[0].children.is_empty());
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
                    name: "test".to_string(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(root);

        let packet = get_command_packet(&world, root.id(), None);

        assert_eq!(packet.commands.len(), 2);
        assert_eq!(packet.root_index, VarInt(0));
        assert_eq!(packet.commands[0].children, vec![VarInt(1)]);
        assert_eq!(packet.commands[1].data, NodeData::Literal {
            name: "test".to_string(),
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
                    name: "parent".to_string(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(root);

        let _child = world
            .entity()
            .set(Command {
                data: NodeData::Literal {
                    name: "child".to_string(),
                },
                has_permission: |_: _, _: _| true,
            })
            .child_of(parent);

        let packet = get_command_packet(&world, root.id(), None);

        assert_eq!(packet.commands.len(), 3);
        assert_eq!(packet.root_index, VarInt(0));
        assert_eq!(packet.commands[0].children, vec![VarInt(1)]);
        assert_eq!(packet.commands[1].children, vec![VarInt(2)]);
        assert_eq!(packet.commands[1].data, NodeData::Literal {
            name: "parent".to_string(),
        });
        assert_eq!(packet.commands[2].data, NodeData::Literal {
            name: "child".to_string(),
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

        let packet = get_command_packet(&world, root.id(), None);

        assert_eq!(packet.commands.len(), MAX_DEPTH + 1);
    }
}
