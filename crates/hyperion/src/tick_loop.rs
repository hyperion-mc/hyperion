//! Everything flecs's own `ecs_app_run` does to a world before it starts ticking, for a
//! host that ticks the world itself.
//!
//! # Why a host would want its own loop
//!
//! `App::run` is `ecs_app_run`, and it does not return until the world quits. A host that
//! has to do anything at all between two frames -- hot reload is the case this exists for,
//! because a module swap mid-frame rebuilds a system table underneath an iterator -- cannot
//! use it. See `hyperion_hot_reload::service::run`, which is that loop.
//!
//! # What `ecs_app_run` actually does, and what is left here
//!
//! Read from flecs's `addons/app.c` at the pinned `flecs_ecs_sys` version, in order:
//!
//! | `ecs_app_run` | here |
//! | --- | --- |
//! | `ecs_set_target_fps(world, desc->target_fps)` | [`prepare`] refuses a world with none |
//! | `ecs_set_threads(world, desc->threads)` | [`HyperionCore`](crate::HyperionCore) |
//! | `ECS_IMPORT(FlecsRest)` + `ecs_set(EcsWorld, EcsRest, {port})` | [`prepare`] |
//! | `ECS_IMPORT(FlecsStats)` | [`prepare`] |
//! | `while (ecs_progress(world, 0)) {}` | the host's loop |
//!
//! The two rows that are not here are not omissions:
//!
//! **Threads.** `HyperionCore` already calls `world.set_threads(rayon::current_num_threads())`
//! while it is being imported, which is before any event's `init_game` reaches its loop.
//! flecs's `flecs_set_threads_internal` returns without doing anything when the stage count
//! already equals the requested thread count, so the `App::set_threads(...)` every event
//! used to call with the same expression was provably a second no-op call rather than the
//! one that mattered.
//!
//! **Target frame rate.** `HyperionCore` sets it to `TICKS_PER_SECOND`. `App::new` would
//! have read that same value back out of the world and set it again -- and, had nothing set
//! one, would have quietly substituted 60. That substitution is the reason [`prepare`]
//! refuses instead of defaulting: a game server whose target rate is zero does not run
//! slowly or quickly, it spins a core per stage as fast as `ecs_progress` will return, and
//! the only outward symptom is a host that is hot.

use anyhow::ensure;
use flecs_ecs::{addons::stats::Stats, core::World, prelude::*};

/// Bring the world to the state `App::enable_rest(0).enable_stats(true).run()` left it in,
/// short of the loop itself.
///
/// The REST port is flecs's own default (27750) rather than a number chosen here, which is
/// what `enable_rest(0)` meant: `desc.port = 0` reaches `EcsRest` as zero, and flecs reads
/// zero as "unset".
///
/// # Errors
/// If no target frame rate has been set on the world -- see the module docs for why that is
/// worth failing over rather than filling in.
pub fn prepare(world: &World) -> anyhow::Result<()> {
    let target_fps = world.info().target_fps;
    ensure!(
        target_fps > 0.0,
        "no target frame rate on this world: import HyperionCore before preparing the tick loop, \
         or the server will spin instead of tick"
    );

    // Both of these are what the flecs Explorer connects to. `FlecsRest` itself is already
    // imported by `ecs_init`, so setting the singleton is the whole of `enable_rest`;
    // `FlecsStats` is not, and is the whole of `enable_stats`.
    world.set(flecs::rest::Rest::default());
    world.import::<Stats>();

    Ok(())
}
