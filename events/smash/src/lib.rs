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
pub mod chat;
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

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::Context;
use flecs_ecs::prelude::*;
use hyperion::{Crypto, GameServerEndpoint, HyperionCore};
use hyperion_clap::hyperion_command::CommandRegistry;
use hyperion_event_runner::Deployment;
use hyperion_hot_reload::service::{self, Outcome, ReloadService};

use crate::{
    module::{
        ability::AbilityModule,
        arena::ArenaModule,
        build_stamp::{self, BuildStamp, BuildStampModule},
        damage::DamageModule,
        effect::EffectModule,
        hud::HudModule,
        jump::JumpModule,
        kit::KitModule,
        kits::StockKits,
        knockback::KnockbackModule,
        lives::LivesModule,
        lobby::LobbyModule,
        player::PlayerModule,
        projectile::ProjectileModule,
        scoreboard::ScoreboardModule,
        selector::SelectorModule,
        sound::SoundModule,
        vitals::VitalsModule,
    },
    server::{NamedColor, Text, Title},
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
        // Chat: hyperion decodes it and broadcasts nothing, so without this a
        // player's message reaches no one. Host-side because it reads a
        // hyperion event queue; see `crate::chat`.
        world.import::<crate::chat::SmashChatModule>();
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

/// Say what the reload did, in the journal and on every player's screen.
///
/// # Why the host does the logging
///
/// `hyperion-hot-reload` reports rather than logs, because it cannot depend on `tracing`
/// without duplicating it against `hyperion`'s copy -- see that crate's `Cargo.toml`. So
/// the severities are chosen here, and they are not the same for the two arms. **A refusal
/// is an `error`**: it means the deploy did not take, and the operator has to find it in a
/// query filtered by severity rather than by reading a log. An accepted reload is `info`,
/// which is what `hyperion-event-runner` defaults to precisely so this line arrives.
///
/// # Why the players are told at all
///
/// A reload is invisible from a client: nothing disconnects, nothing respawns, and the
/// only outward sign is that a rule behaves differently than it did a second ago. Somebody
/// in a match who is about to lose to a number that just changed deserves to know it
/// changed, and somebody watching their own deploy land needs the confirmation that it
/// landed. A title is the one channel that reaches a player looking at the middle of their
/// screen, which in a fighting game is everyone.
///
/// The build bar underneath is not a duplicate of it: the title is gone in a few seconds
/// and says "something just changed", the bar stays and says "to what".
///
/// # Why this is a callback and not an observer
///
/// A reload deletes and re-creates every system and observer the module registered, so an
/// observer is exactly the wrong shape -- the thing that would react to the event is the
/// thing the event just replaced. See `hyperion_hot_reload::service::run`, which calls this
/// between two frames.
fn announce_reload(world: &World, outcome: &Outcome, build_stamp: &Path) {
    let reloaded = match outcome {
        Outcome::Applied(reloaded) => reloaded,
        Outcome::Refused(reason) => {
            tracing::error!("hot reload refused, the world is unchanged: {reason}");
            return;
        }
    };

    // Re-read, because the deploy that carried these rules rewrote these files and there is
    // no new `exec` to pick them up. This is the whole reason the stamp is files rather
    // than the environment it used to be.
    world.set(BuildStamp::read(build_stamp));

    // `reloaded.revision` is `<build_stamp>/build-rev` read by the service on the same
    // reload, so the title and the bar cannot name different builds.
    let label = reloaded.revision.as_deref().unwrap_or("an unknown build");

    tracing::info!(
        module = %reloaded.module,
        revision = label,
        migrated_instances = reloaded.migrated_instances,
        "hot reload accepted"
    );

    world.get::<&ServerHandle>(|server| {
        server.broadcast_title(Title::new(
            Text::text(format!("Reloading to build {label}")).color(NamedColor::Yellow),
        ));
    });
}

/// The reload half of a deployment, bound and loaded.
///
/// The two travel together because [`announce_reload`] needs the stamp
/// directory and only a server that has a service can ever reach it.
struct Rules {
    service: ReloadService,
    build_stamp: PathBuf,
}

impl Rules {
    /// Bind the socket and load the rules for the first time.
    ///
    /// A first load that fails is fatal, unlike a reload that is refused: the
    /// running world has nothing to protect yet, and a server that came up
    /// silently missing every one of its rules is worse than one that did not
    /// come up.
    fn open(world: &World, deployment: Deployment) -> anyhow::Result<Self> {
        world.set(BuildStamp::read(&deployment.build_stamp));

        let mut service = ReloadService::bind(
            &deployment.reload_socket,
            deployment.rules.clone(),
            deployment.build_stamp.join(build_stamp::REV_FILE),
        )
        .with_context(|| format!("binding {}", deployment.reload_socket.display()))?;

        let applied = service
            .load_initial(world)
            // `LoadError` is not `Send + Sync`, which `anyhow::Error` wants:
            // it carries a `libloading::Error`. The text is the whole value.
            .map_err(|e| anyhow::anyhow!("loading {}: {e}", deployment.rules.display()))?;
        tracing::info!(module = %applied.module, "rules loaded");

        Ok(Self {
            service,
            build_stamp: deployment.build_stamp,
        })
    }
}

/// Build the world and run it. The entry point `main.rs` calls.
///
/// `deployment` is the packaged shape: a rules dylib to load, a socket to be
/// asked to load it again on, and the directory the build stamp is written to.
/// `None` is a developer's server or an end-to-end gate -- same world, same
/// loop, no reloadable rules.
///
/// # Why this does not call `App::run`
///
/// It used to. `App::run` is flecs's `ecs_app_run`, which does not return until
/// the world quits, and a reload has to happen between two frames: swapping a
/// module mid-frame rebuilds a system table underneath an iterator. So the host
/// ticks the world itself. [`hyperion::tick_loop::prepare`] is what
/// `ecs_app_run` did to the world before its own loop, and
/// [`hyperion_hot_reload::service::run`] is the loop.
///
/// # Errors
/// If the world has no target frame rate, if the reload socket cannot be bound,
/// or if the rules dylib is refused on the first load. A rules dylib that
/// cannot be loaded at startup is fatal, unlike one refused later: the running
/// world has nothing to protect yet, and a server that came up silently missing
/// every one of its rules is worse than one that did not come up.
pub fn init_game(
    address: SocketAddr,
    crypto: Crypto,
    deployment: Option<Deployment>,
) -> anyhow::Result<()> {
    let world = World::new();

    world.import::<HyperionCore>();
    world.import::<SmashHost>();

    world.set(crypto);
    world.set(GameServerEndpoint::from(address));

    let mut rules = deployment.map(|it| Rules::open(&world, it)).transpose()?;

    hyperion::tick_loop::prepare(&world)?;

    match rules.as_mut() {
        // A developer's server or an end-to-end gate: the same world and the
        // same loop, with nothing to reload. The callback cannot be reached
        // without a service, so it has nothing to do.
        None => service::run(&world, None, |_, _| {}),
        Some(rules) => {
            let build_stamp = rules.build_stamp.clone();
            service::run(&world, Some(&mut rules.service), |world, outcome| {
                announce_reload(world, outcome, &build_stamp);
            });
        }
    }

    Ok(())
}
