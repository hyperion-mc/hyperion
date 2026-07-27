#![feature(iter_intersperse)]

use flecs_ecs::{core::World, macros::Component, prelude::Module};

mod component;
mod system;

pub use component::{CommandHandler, CommandRegistry};

#[derive(Component)]
pub struct CommandModule;

impl Module for CommandModule {
    fn module(world: &World) {
        world.import::<component::CommandComponentModule>();
        world.import::<system::CommandSystemModule>();
    }
}
