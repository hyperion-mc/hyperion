use bevy::prelude::*;

mod component;
mod system;

pub use component::{CommandHandler, CommandRegistry, ExecutableCommand};

pub struct CommandPlugin;

impl Plugin for CommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            component::CommandComponentPlugin,
            system::CommandSystemPlugin,
        ));
    }
}
