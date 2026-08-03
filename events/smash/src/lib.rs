//! Mineplex Super Smash Mobs for the hyperion Minecraft engine.
//!
//! The game is an import list. Every subsystem and every kit is a flecs
//! [`Module`], so a deployment picks its feature set by choosing which modules
//! to import and a new kit is a new file plus one import line.
//!
//! [`SmashModule`] is the game and knows nothing about a host. [`SmashHost`] is
//! the game plus hyperion, and everything hyperion-shaped lives under it in
//! [`adapter`], [`mirror`], [`input`] and [`command`].

pub mod adapter;
pub mod command;
pub mod draw;
pub mod flecs_ext;
pub mod input;
pub mod map;
pub mod mirror;
pub mod module;
pub mod server;
pub mod terrain;
pub mod terrain_seam;

use std::net::SocketAddr;

use flecs_ecs::prelude::*;
use hyperion::{Crypto, GameServerEndpoint, HyperionCore};
use hyperion_clap::hyperion_command::CommandRegistry;

use crate::module::{
    ability::AbilityModule, arena::ArenaModule, build_stamp::BuildStampModule,
    damage::DamageModule, effect::EffectModule, hud::HudModule, jump::JumpModule, kit::KitModule,
    kits::StockKits, knockback::KnockbackModule, lives::LivesModule, lobby::LobbyModule,
    player::PlayerModule, projectile::ProjectileModule, scoreboard::ScoreboardModule,
    selector::SelectorModule, sound::SoundModule, vitals::VitalsModule,
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
        // Before the abilities and the kits: both declare sound relationships,
        // and a relationship used before it is registered is not the one the
        // module later configures.
        world.import::<SoundModule>();
        world.import::<AbilityModule>();
        // Before the kits, which hand it afflictions, and after `DamageModule`,
        // whose `MatchClock` every effect's deadline is measured against.
        world.import::<EffectModule>();
        world.import::<KitModule>();
        // After the kits, whose `apply` is what puts a regeneration rate and a
        // hunger interval on a player in the first place.
        world.import::<VitalsModule>();
        world.import::<ArenaModule>();
        world.import::<LivesModule>();
        // After `Lives`: the double jump reads the two components that say a
        // player is spectating rather than playing, so that it can leave them
        // alone.
        world.import::<JumpModule>();
        world.import::<ProjectileModule>();
        world.import::<LobbyModule>();
        world.import::<ScoreboardModule>();
        world.import::<HudModule>();
        world.import::<SelectorModule>();
        world.import::<StockKits>();
        // Last, and that placement is load bearing: flecs runs same-phase
        // systems in the order they were declared, so declaring this after
        // `HudModule`'s `update_hud` is what puts the match bar above the
        // build stamp on a joining player's screen rather than below it.
        world.import::<BuildStampModule>();
    }
}

/// The game, hosted on hyperion.
///
/// Separate from [`SmashModule`] so the game stays testable in a bare world:
/// every test under `tests/` imports the game and a mock seam, and neither of
/// them pays for a Minecraft server.
#[derive(Component)]
pub struct SmashHost;

impl Module for SmashHost {
    fn module(world: &World) {
        world.import::<hyperion_utils::HyperionUtilsModule>();
        world.import::<hyperion_permission::PermissionModule>();
        world.import::<hyperion_clap::ClapCommandModule>();
        // Registers a handler for hyperion's `InteractEvent`. Without one, every
        // right-click logs "No handlers registered" and the packet is dropped
        // before the ability layer ever sees it.
        world.import::<hyperion_item::ItemModule>();

        world.import::<crate::adapter::SmashAdapterModule>();
        // After the adapter, because building the maps writes the `Arena`
        // singleton the game half registered.
        world.import::<crate::terrain::MapModule>();
        // Points the game's terrain reads at hyperion's block store. Without
        // it the game half keeps its `OpenAir` default and projectiles fly
        // through the arena, which is what they did before this existed.
        world.import::<crate::terrain_seam::TerrainSeamModule>();
        // After the adapter too: it draws the game half's projectiles, whose
        // `Projectile`, `Visual` and `Flight` the adapter's `SmashModule`
        // import is what registers.
        world.import::<crate::draw::DrawModule>();

        world.get::<&mut CommandRegistry>(|registry| {
            command::register(registry, world);
        });
    }
}

/// Build the world and run it. The entry point `main.rs` calls.
///
/// `embedded_proxy` asks for a proxy inside this process, listening on that
/// address. Running one is the shortest path to a playable server, but it is
/// optional because the deployed shape is proxies on their own machines --
/// and because two proxies racing for one port is the sort of thing that
/// leaves a dev stack half up with no obvious reason why.
///
/// # Errors
/// If the thread count does not fit in the `i32` flecs wants.
pub fn init_game(address: SocketAddr, crypto: Crypto) -> anyhow::Result<()> {
    let world = World::new();

    world.import::<HyperionCore>();
    world.import::<SmashHost>();

    world.set(crypto);
    world.set(GameServerEndpoint::from(address));

    let mut app = world.app();

    app.enable_rest(0)
        .enable_stats(true)
        .set_threads(i32::try_from(rayon::current_num_threads())?);

    app.run();

    Ok(())
}
