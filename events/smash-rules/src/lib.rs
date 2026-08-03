//! smash's reloadable rules.
//!
//! This crate is the unit of hot reload: its store path is what
//! `nix/modules/game-server.nix` puts in `X-Reload-Triggers`, so an edit here
//! reaches a running server as `systemctl reload` and an edit anywhere else
//! reaches it as a restart. That boundary is the source split and nothing else
//! -- see `docs/hot-reload.md`, "Packaging: three derivations".
//!
//! # What may live here, and what may not
//!
//! Systems and observers only. **No `world.component::<T>()`.** A component's
//! layout has to have exactly one definition in the process, and that
//! definition lives in the host, because a system compiled against one layout
//! reading a world that holds another is silent memory corruption rather than
//! an error. The host registers; this crate reads and writes.
//!
//! The gate in `hyperion-hot-reload` enforces the consequence rather than the
//! rule: it refuses a reload whose component schemas moved. Keeping
//! registration out of here is what makes that refusal never fire in normal
//! work.
//!
//! # Deliberately trivial, for now
//!
//! One system that logs. The deployment half -- reload-not-restart, the
//! surviving player connection, the title -- is being proven against this
//! before smash's real rules are migrated into it, because the refactor has no
//! unknowns and the deployment has all of them. `docs/hot-reload.md` explains
//! why that order is the reverse of the order they were designed in.

use flecs_ecs::prelude::*;

/// How many ticks between heartbeat lines.
///
/// A whole second at 20 tps. Frequent enough that a reload's effect shows up in
/// the journal while somebody is watching for it, rare enough that it is not
/// the reason the journal fills.
const TICKS_PER_BEAT: u64 = 20;

/// What the heartbeat says.
///
/// Editing this string is the canonical rules-only change: it moves this
/// crate's store path and nothing else's, so the deploy that carries it must
/// reload rather than restart. The reload proof drives exactly this edit.
const BEAT: &str = "smash rules heartbeat LIVE-RELOAD-1";

/// Behaviour, and no registration. See the module docs for why that split is
/// load bearing rather than tidy.
#[derive(Component)]
pub struct SmashRulesModule;

impl Module for SmashRulesModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Rules");

        // Counted in the closure rather than read from a component, because a
        // component would be registration and registration belongs to the
        // host. A `static` inside the dylib is re-initialised by each new
        // build, which is the honest behaviour for a counter that is not game
        // state: nothing migrates it and nothing should.
        let mut ticks: u64 = 0;
        world
            .system_named::<()>("smash_rules_heartbeat")
            .kind(id::<flecs::pipeline::OnUpdate>())
            .run(move |mut it| {
                while it.next() {
                    ticks += 1;
                    if ticks.is_multiple_of(TICKS_PER_BEAT) {
                        tracing::info!("{BEAT}");
                    }
                }
            });
    }
}

hyperion_hot_reload::export_module! {
    name: "smash-rules",
    register: |world| {
        world.import::<SmashRulesModule>();
    },
}
