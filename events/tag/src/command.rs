use flecs_ecs::core::World;
use hyperion_clap::{ MinecraftCommand, hyperion_command::CommandRegistry };

use crate::command::{
    bow::BowCommand,
    class::ClassCommand,
    fly::FlyCommand,
    gui::GuiCommand,
    raycast::RaycastCommand,
    replace::ReplaceCommand,
    shoot::ShootCommand,
    spawn::SpawnCommand,
    speed::SpeedCommand,
    vanish::VanishCommand,
    xp::XpCommand,
    chest::ChestCommand,
};

mod bow;
mod class;
mod fly;
mod gui;
mod raycast;
mod replace;
mod shoot;
mod spawn;
mod speed;
mod vanish;
mod xp;
mod chest;

pub fn register(registry: &mut CommandRegistry, world: &World) {
    BowCommand::register(registry, world);
    ClassCommand::register(registry, world);
    FlyCommand::register(registry, world);
    GuiCommand::register(registry, world);
    RaycastCommand::register(registry, world);
    ReplaceCommand::register(registry, world);
    ShootCommand::register(registry, world);
    SpawnCommand::register(registry, world);
    SpeedCommand::register(registry, world);
    VanishCommand::register(registry, world);
    XpCommand::register(registry, world);
    ChestCommand::register(registry, world);
}
