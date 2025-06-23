use bevy::prelude::*;
use hyperion_clap::MinecraftCommand;

use crate::command::{
    bow::BowCommand,
    // chest::ChestCommand,
    class::ClassCommand,
    fly::FlyCommand,
    // gui::GuiCommand,
    raycast::RaycastCommand,
    replace::ReplaceCommand,
    shoot::ShootCommand,
    spawn::SpawnCommand,
    speed::SpeedCommand,
    // vanish::VanishCommand,
    xp::XpCommand,
};

mod bow;
// mod chest;
mod class;
mod fly;
// mod gui;
mod raycast;
mod replace;
mod shoot;
mod spawn;
mod speed;
// mod vanish;
mod xp;

pub fn register(world: &mut World) {
    BowCommand::register(world);
    ClassCommand::register(world);
    FlyCommand::register(world);
    // GuiCommand::register(world);
    RaycastCommand::register(world);
    ReplaceCommand::register(world);
    ShootCommand::register(world);
    SpawnCommand::register(world);
    SpeedCommand::register(world);
    // VanishCommand::register(world);
    XpCommand::register(world);
    // ChestCommand::register(world);
}
