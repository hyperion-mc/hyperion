use std::sync::Mutex;

use bevy::prelude::*;
use derive_more::{Deref, DerefMut};
use hyperion::simulation::packet::play;
use hyperion_utils::ApplyWorld;
use indexmap::IndexMap;

pub trait ExecutableCommand: ApplyWorld {
    /// Executes a command triggered by a player
    fn execute(&mut self, world: &World, execution: &play::CommandExecution);
}

pub struct CommandHandler {
    pub executable: Box<dyn ExecutableCommand + Send + Sync + 'static>,
    pub tab_complete: fn(&World, &play::RequestCommandCompletions),
    pub has_permissions: fn(&World, Entity) -> bool,
}

pub struct CommandRegistryInner {
    pub(crate) commands: IndexMap<String, CommandHandler>,
}

impl CommandRegistryInner {
    pub fn register(&mut self, name: impl Into<String>, handler: CommandHandler) {
        let name = name.into();
        self.commands.insert(name, handler);
    }

    pub fn all(&self) -> impl Iterator<Item = &str> {
        self.commands.keys().map(String::as_str)
    }

    /// Returns an iterator over the names of commands (`&str`) that the given entity (`caller`)
    /// has permission to execute.
    pub fn get_permitted(&self, world: &World, caller: Entity) -> impl Iterator<Item = &str> {
        self.commands
            .iter()
            .filter_map(move |(cmd_name, handler)| {
                if (handler.has_permissions)(world, caller) {
                    Some(cmd_name)
                } else {
                    None
                }
            })
            .map(String::as_str)
    }
}

/// Registry storing a list of commands.
///
/// This registry is locked by a [`Mutex`]. See the `execute_commands` system for justification.
/// Consider accessing this resource using [`ResMut`] and [`Mutex::get_mut`].
#[derive(Resource, Deref, DerefMut)]
pub struct CommandRegistry(Mutex<CommandRegistryInner>);

pub struct CommandComponentPlugin;

impl Plugin for CommandComponentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CommandRegistry(Mutex::new(CommandRegistryInner {
            commands: IndexMap::default(),
        })));
    }
}
