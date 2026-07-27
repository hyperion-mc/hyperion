//! Mineplex Super Smash Mobs for the hyperion Minecraft engine.
//!
//! The game is an import list. Every subsystem and every kit is a flecs
//! [`Module`], so a deployment picks its feature set by choosing which modules
//! to import and a new kit is a new file plus one import line.

pub mod flecs_ext;
pub mod module;
pub mod server;

use flecs_ecs::prelude::*;

use crate::module::{
    ability::AbilityModule, arena::ArenaModule, damage::DamageModule, kit::KitModule,
    kits::StockKits, knockback::KnockbackModule, lives::LivesModule, lobby::LobbyModule,
    player::PlayerModule, projectile::ProjectileModule, scoreboard::ScoreboardModule,
};

/// The whole game.
#[derive(Component)]
pub struct SmashModule;

use crate::server::{PlayerId, ServerHandle};

impl Module for SmashModule {
    fn module(world: &World) {
        world.module::<Self>("smash");

        // The workspace enables flecs_manual_registration, so every component
        // must be registered before first use. ServerHandle belongs to no
        // submodule -- it is the seam the host installs -- so it registers here.
        world.component::<ServerHandle>();
        world.component::<PlayerId>();

        world.import::<PlayerModule>();
        world.import::<KnockbackModule>();
        world.import::<DamageModule>();
        world.import::<AbilityModule>();
        world.import::<KitModule>();
        world.import::<ArenaModule>();
        world.import::<LivesModule>();
        world.import::<ProjectileModule>();
        world.import::<LobbyModule>();
        world.import::<ScoreboardModule>();
        world.import::<StockKits>();
    }
}
