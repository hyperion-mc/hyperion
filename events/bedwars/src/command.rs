use flecs_ecs::core::World;
use hyperion_clap::{MinecraftCommand, hyperion_command::CommandRegistry};

use crate::command::{
    bow::BowCommand, chest::ChestCommand, fly::FlyCommand, gui::GuiCommand,
    raycast::RaycastCommand, shoot::ShootCommand, speed::SpeedCommand, vanish::VanishCommand,
    xp::XpCommand,
};

mod bow;
mod chest;
mod fly;
mod gui;
mod raycast;
mod shoot;
mod speed;
mod vanish;
mod xp;

pub fn register(registry: &mut CommandRegistry, world: &World) {
    BowCommand::register(registry, world);
    FlyCommand::register(registry, world);
    GuiCommand::register(registry, world);
    RaycastCommand::register(registry, world);
    ShootCommand::register(registry, world);
    SpeedCommand::register(registry, world);
    VanishCommand::register(registry, world);
    XpCommand::register(registry, world);
    ChestCommand::register(registry, world);
}
