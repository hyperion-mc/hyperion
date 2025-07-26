use bevy::prelude::*;
use hyperion_clap::MinecraftCommand;

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

pub fn register(world: &mut World) {
    BowCommand::register(world);
    FlyCommand::register(world);
    GuiCommand::register(world);
    RaycastCommand::register(world);
    ShootCommand::register(world);
    SpeedCommand::register(world);
    VanishCommand::register(world);
    XpCommand::register(world);
    ChestCommand::register(world);
}
