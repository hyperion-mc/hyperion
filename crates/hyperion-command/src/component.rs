use flecs_ecs::{
    core::{Entity, EntityView, World},
    macros::Component,
    prelude::Module,
};
use hyperion::{simulation::handlers::PacketSwitchQuery, storage::CommandCompletionRequest};
use indexmap::IndexMap;

pub type OnTabComplete =
    Box<dyn Fn(&mut PacketSwitchQuery<'_>, &CommandCompletionRequest<'_>) + 'static + Send + Sync>;
pub struct CommandHandler {
    pub on_execute: fn(input: &str, system: EntityView<'_>, caller: Entity),
    pub on_tab_complete: OnTabComplete,
    pub has_permissions: fn(world: &World, caller: Entity) -> bool,
}

#[derive(Component)]
pub struct CommandRegistry {
    pub(crate) commands: IndexMap<String, CommandHandler>,
}

impl CommandRegistry {
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

#[derive(Component)]
pub struct CommandComponentModule;

impl Module for CommandComponentModule {
    fn module(world: &World) {
        world.component::<CommandRegistry>();
        world.set(CommandRegistry {
            commands: IndexMap::default(),
        });
    }
}
