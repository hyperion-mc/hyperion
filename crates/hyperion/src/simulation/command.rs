use flecs_ecs::{
    core::{Entity, EntityViewGet, IdOperations, World},
    macros::Component,
};
use rustc_hash::FxHashMap;
use tracing::warn;
pub use valence_protocol::packets::play::command_tree_s2c::Parser;
use valence_protocol::{
    VarInt,
    packets::play::command_tree_s2c::{Node, NodeData},
};

#[derive(Component)]
pub struct Command {
    data: NodeData,
}

#[allow(clippy::must_use_candidate)]
pub fn add_command(world: &World, command: Command, parent: Entity) -> Entity {
    world.entity().set(command).child_of_id(parent).id()
}

/// Entry point for defining commands using the DSL.
pub fn cmd_with<'a, F>(world: &'a World, name: &str, f: F)
where
    F: FnOnce(&mut CommandScope<'a>),
{
    let mut scope = CommandScope::new(world);
    scope.literal_with(name, f);
}

pub fn cmd<F>(world: &World, name: &str) {
    cmd_with(world, name, |_| {});
}

#[must_use]
pub struct CommandScope<'a> {
    world: &'a World,
    current: Entity,
    parents: FxHashMap<Entity, Entity>,
}

impl<'a> CommandScope<'a> {
    fn new(world: &'a World) -> Self {
        Self {
            world,
            current: *ROOT_COMMAND.get().expect("Root command not initialized"),
            parents: FxHashMap::default(),
        }
    }

    /// Adds a literal command. Accepts an optional closure to define nested commands.
    pub fn literal_with<F>(&mut self, name: &str, f: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
    {
        let command = Command::literal(name);
        let entity = add_command(self.world, command, self.current);
        self.parents.insert(entity, self.current);
        self.current = entity;

        // Execute the closure to define nested commands
        f(self);

        // Return to the parent command
        self.end()
    }

    pub fn literal(&mut self, name: &str) -> &mut Self {
        self.literal_with(name, |_| {})
    }

    pub fn argument(&mut self, name: &str, parser: Parser) -> &mut Self {
        self.argument_with(name, parser, |_| {})
    }

    /// Adds an argument command. Accepts an optional closure to define nested commands.
    pub fn argument_with(
        &mut self,
        name: &str,
        parser: Parser,
        f: impl FnOnce(&mut Self),
    ) -> &mut Self {
        let command = Command::argument(name, parser);
        let entity = add_command(self.world, command, self.current);
        self.parents.insert(entity, self.current);
        self.current = entity;

        // Execute the closure to define nested commands
        f(self);

        // Return to the parent command
        self.end()
    }

    /// Ends the current command scope, returning to the parent command.
    pub fn end(&mut self) -> &mut Self {
        if let Some(parent) = self.parents.get(&self.current).copied() {
            self.current = parent;
        }
        self
    }
}

pub(crate) static ROOT_COMMAND: once_cell::sync::OnceCell<Entity> =
    once_cell::sync::OnceCell::new();

pub fn get_root_command() -> Entity {
    *ROOT_COMMAND.get().unwrap()
}

impl Command {
    pub const ROOT: Self = Self {
        data: NodeData::Root,
    };

    #[must_use]
    pub fn literal(name: &str) -> Self {
        Self {
            data: NodeData::Literal {
                name: name.to_string(),
            },
        }
    }

    #[must_use]
    pub fn argument(name: &str, parser: Parser) -> Self {
        Self {
            data: NodeData::Argument {
                name: name.to_string(),
                parser,
                suggestion: None,
            },
        }
    }
}

// we want a get command packet

const MAX_DEPTH: usize = 64;

pub fn get_command_packet(
    world: &World,
    root: Entity,
) -> valence_protocol::packets::play::CommandTreeS2c {
    struct StackElement {
        depth: usize,
        ptr: usize,
        entity: Entity,
    }

    let mut commands = Vec::new();

    let mut stack = vec![StackElement {
        depth: 0,
        ptr: 0,
        entity: root,
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

        world.entity_from_id(entity).each_child(|child| {
            child.get::<&Command>(|command| {
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
                    entity: child.id(),
                });
            });
        });
    }

    valence_protocol::packets::play::CommandTreeS2c {
        commands,
        root_index: VarInt(0),
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

        let packet = get_command_packet(&world, root.id());

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
            })
            .child_of_id(root);

        let packet = get_command_packet(&world, root.id());

        assert_eq!(packet.commands.len(), 2);
        assert_eq!(packet.root_index, VarInt(0));
        assert_eq!(packet.commands[0].children, vec![VarInt(1)]);
        assert_eq!(packet.commands[1].data, NodeData::Literal {
            name: "test".to_string()
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
            })
            .child_of_id(root);

        let _child = world
            .entity()
            .set(Command {
                data: NodeData::Literal {
                    name: "child".to_string(),
                },
            })
            .child_of_id(parent);

        let packet = get_command_packet(&world, root.id());

        assert_eq!(packet.commands.len(), 3);
        assert_eq!(packet.root_index, VarInt(0));
        assert_eq!(packet.commands[0].children, vec![VarInt(1)]);
        assert_eq!(packet.commands[1].children, vec![VarInt(2)]);
        assert_eq!(packet.commands[1].data, NodeData::Literal {
            name: "parent".to_string()
        });
        assert_eq!(packet.commands[2].data, NodeData::Literal {
            name: "child".to_string()
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
                })
                .child_of_id(parent);
            parent = child;
        }

        let packet = get_command_packet(&world, root.id());

        assert_eq!(packet.commands.len(), MAX_DEPTH + 1);
    }
}
