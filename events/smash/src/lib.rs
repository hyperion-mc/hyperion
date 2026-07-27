//! Mineplex Super Smash Mobs for the hyperion Minecraft engine.
//!
//! The game is an import list. Every subsystem and every kit is a flecs
//! [`Module`], so a deployment picks its feature set by choosing which modules
//! to import and a new kit is a new file plus one import line.

pub mod module;
pub mod server;

use flecs_ecs::prelude::*;

use crate::module::{damage::DamageModule, knockback::KnockbackModule, player::PlayerModule};

/// The whole game.
#[derive(Component)]
pub struct SmashModule;

impl Module for SmashModule {
    fn module(world: &World) {
        world.module::<Self>("smash");
        world.import::<PlayerModule>();
        world.import::<KnockbackModule>();
        world.import::<DamageModule>();
    }
}
