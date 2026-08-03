//! What build the server is running, on the player's screen.
//!
//! There is a live server that is redeployed as main moves, and until this
//! existed the only way to tell whether a change had reached it was to guess at
//! deploy timing from outside the game. The commit and how long ago it was made
//! are the two facts that answer it, so they go where a player is already
//! looking.
//!
//! # Why a second bar and not the lobby's
//!
//! The other candidate was [`crate::module::hud::boss_bar`]'s `Phase::Waiting`
//! arm -- the "Waiting for players 1/2" strip -- and it was rejected because
//! that bar is **not always there**. It becomes a countdown, then a percentage,
//! the moment a match starts, so a stamp folded into it answers the question
//! only for somebody who happens to arrive between matches. The question is
//! asked at arbitrary times, including by a person who joined a running match
//! to check, and half the time the lobby bar is a bar about something else.
//!
//! What its own bar gives up is a permanent strip of screen in a fighting game,
//! which is a real cost. It is paid down as far as it goes: the fill is left
//! empty so it draws no coloured length, and it is pushed after the match bar
//! so it sits under it rather than above.
//!
//! # The age is relative now, and here is what that costs
//!
//! This bar used to read `build 8f3a21c · 2026-07-29 18:04 UTC` and was sent
//! exactly once per player, on the argument that a build stamp is constant for
//! the life of a process and should cost exactly one packet per viewer.
//!
//! Two things ended that argument. The absolute form does not answer the
//! question the bar is on screen for -- "is my change live yet" is a question
//! about elapsed time, and a UTC minute has to be subtracted from a clock the
//! reader goes and finds, in a fighting game, mid-match. And the process is no
//! longer the unit anyway: a hot reload replaces the rules without restarting,
//! so "constant for the life of a process" stopped being true the moment
//! [`crate::init_game`] started re-reading this stamp on every accepted reload.
//!
//! So the wording moved to `build 8f3a21c · 2h ago`, and a relative age changes
//! on its own. The cost is bounded by coarsening the wording as the build ages,
//! which is [`relative_age`]'s whole job: a string reading to the minute changes
//! once a minute, `2h ago` changes once an hour, `3d ago` changes once a day.
//! One build therefore costs at most 60 packets per viewer in its first hour,
//! 23 more across the rest of its first day, and one a day after that. The
//! steady state a deployed server sits in is **one packet per viewer per hour**.
//!
//! `show_build_stamp` is what turns that bound into behaviour rather than
//! arithmetic: it sends only when the rendered string differs from the one that
//! player was last sent, so the packet count is a function of the wording and
//! not of the tick rate.
//!
//! # Where the values come from
//!
//! Three files in a directory the deployment names on the command line
//! (`--build-stamp`), written by `nix/modules/game-server.nix`.
//!
//! **Files and not environment variables**, which is what they were until hot
//! reload existed. A process's environment is fixed at `exec`, so a server that
//! reloaded its rules without restarting would go on reporting the build it
//! started as, forever -- and not restarting is the entire point.
//!
//! Not `env!` in a build script either: baking a commit hash into a crate makes
//! every commit a full workspace recompile, and the compile is already the floor
//! on the pipeline.
//!
//! A `cargo run` build is handed no directory at all and says so, rather than
//! claiming a commit it was not built from.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use flecs_ecs::prelude::*;

use crate::{
    module::player::Player,
    server::{BarColour, BarSlot, BossBar, NamedColor, PlayerId, ServerHandle, Text},
};

/// The short commit hash the server was built from, with no dirty marker on it.
/// That marker is [`DIRTY_FILE`]'s job.
///
/// Public because `hyperion_hot_reload::ReloadService` reads this same file to
/// name the build in its `accepted <module> <revision>` reply. One filename with
/// two spellings is one deploy away from a reload reporting a revision nobody
/// wrote.
pub const REV_FILE: &str = "build-rev";

/// When that commit was made, in whole seconds since the unix epoch.
pub const TIME_FILE: &str = "build-time";

/// `1` when the working tree had uncommitted changes at build time.
pub const DIRTY_FILE: &str = "build-dirty";

/// What build this process is currently running.
///
/// Every field is optional because the honest answer for a `cargo run` build is
/// that nobody said. A missing field reads as "unknown" on screen rather than
/// as a default that would be a lie -- a stamp that says `0000000` is worse
/// than one that says it does not know.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildStamp {
    /// The short commit hash, `None` when nothing said.
    pub rev: Option<String>,
    /// When that commit was made, in seconds since the unix epoch.
    pub committed_at: Option<i64>,
    /// The tree had uncommitted changes when this was built, so the commit
    /// above names a tree this binary was not compiled from.
    pub dirty: bool,
}

impl BuildStamp {
    /// What the files in `dir` say this build is.
    ///
    /// A directory that is not there, or a file that cannot be read, is the
    /// unpackaged case rather than an error. This is a label; a server that
    /// refused to start over one would be trading a running game for a string.
    #[must_use]
    pub fn read(dir: &Path) -> Self {
        let field = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        Self::parse(
            field(REV_FILE).as_deref(),
            field(TIME_FILE).as_deref(),
            field(DIRTY_FILE).as_deref(),
        )
    }

    /// The same, from three strings.
    ///
    /// Split out from [`Self::read`] so that every wording case is reachable
    /// without a filesystem.
    ///
    /// A field that is present but unusable -- an empty rev, a time that is not
    /// a number -- is treated as absent. The bar is a readout and half of one is
    /// better than none. Everything is trimmed, because `environment.etc` ends
    /// each of these files with a newline.
    #[must_use]
    pub fn parse(rev: Option<&str>, committed_at: Option<&str>, dirty: Option<&str>) -> Self {
        Self {
            rev: rev
                .map(str::trim)
                .filter(|rev| !rev.is_empty())
                .map(ToOwned::to_owned),
            committed_at: committed_at.map(str::trim).and_then(|at| at.parse().ok()),
            dirty: dirty.map(str::trim) == Some("1"),
        }
    }
}

/// How long ago `committed_at` was, seen from `now`, in as few characters as it
/// can be said in.
///
/// # The granularity is the packet budget
///
/// Each unit is chosen so the string is stable for one whole unit of it, which
/// is what bounds how often the bar is redrawn. Minutes for the first hour,
/// because that is the window somebody watching their own deploy land is
/// actually in; hours for the first day; days after that. Nothing finer than a
/// minute, because a bar that changed every second would cost twenty times more
/// than the whole rest of this module and tell a reader nothing they did not
/// already know.
///
/// A build that has not yet aged a minute reads `just now`, and so does one
/// whose timestamp is in the future. The future case is not hypothetical -- a
/// host whose clock is behind the machine that made the commit produces it --
/// and `in -3m` on a boss bar is a worse answer than a slightly early `just
/// now`.
#[must_use]
pub fn relative_age(committed_at: i64, now: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    let age = now.saturating_sub(committed_at).max(0);
    if age < MINUTE {
        "just now".to_owned()
    } else if age < HOUR {
        format!("{}m ago", age / MINUTE)
    } else if age < DAY {
        format!("{}h ago", age / HOUR)
    } else {
        format!("{}d ago", age / DAY)
    }
}

/// The line across the top of the screen, as plain text.
///
/// Split from [`stamp_bar`] because the colours are a function of one boolean
/// and the wording is the part worth pinning in a test.
#[must_use]
pub fn stamp_title(stamp: &BuildStamp, now: i64) -> String {
    let rev = match (stamp.rev.as_deref(), stamp.dirty) {
        (Some(rev), true) => format!("{rev} + uncommitted changes"),
        (Some(rev), false) => rev.to_owned(),
        // Not "unknown" alone: the reason it is unknown is the useful half,
        // because it tells a developer looking at their own `cargo run` that
        // nothing is broken.
        (None, _) => "unpackaged build".to_owned(),
    };
    stamp.committed_at.map_or_else(
        || format!("build {rev}"),
        |at| format!("build {rev} \u{b7} {}", relative_age(at, now)),
    )
}

/// The bar a build stamp draws.
///
/// **Empty, and that is deliberate.** A boss bar's fill is a fraction of
/// something, and a build is not a fraction of anything; leaving it at zero
/// means the strip draws its frame and its text and no coloured length, which
/// is the least ink this can cost on a screen somebody is fighting on.
///
/// **Red when the tree was dirty.** A dirty build is not the commit it names,
/// and the whole value of a stamp is that it can be trusted, so the one case
/// where it cannot be is the one case that is impossible to skim past.
#[must_use]
pub fn stamp_bar(stamp: &BuildStamp, now: i64) -> BossBar {
    BossBar {
        title: Text::text(stamp_title(stamp, now)).color(if stamp.dirty {
            NamedColor::Red
        } else {
            NamedColor::Gray
        }),
        progress: 0.0,
        colour: if stamp.dirty {
            BarColour::Red
        } else {
            BarColour::Blue
        },
    }
}

/// Wall-clock seconds since the unix epoch.
///
/// Wall clock and not the world's own tick counter, because the number this is
/// compared against is a git commit's timestamp, which lives on the same clock.
/// A host whose clock is wrong renders an age that is wrong by the same amount,
/// which is the honest failure for a readout of somebody else's timestamp.
fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The last thing this player was told about the build.
///
/// It holds the rendered string rather than a tag, and that is what makes the
/// system idempotent instead of edge-triggered: "have they been told" is a
/// question with a stale answer the moment the wording changes, and the wording
/// now changes on its own. Comparing against what was actually sent means a
/// player who joined ten seconds ago and a player who has been standing there
/// since the deploy are handled by one branch.
#[derive(Component, Debug)]
pub struct StampShown {
    /// Exactly the text of the bar this player last received.
    pub text: String,
}

/// Registration: the types this file owns.
///
/// The stamp singleton is registered here and left at its default. Filling it in
/// is [`crate::init_game`]'s job, because only the host knows whether it was
/// handed a `--build-stamp` directory -- and it re-fills it on every accepted
/// reload, which is a thing a registration module could not do.
#[derive(Component)]
pub struct BuildStampComponentsModule;

impl Module for BuildStampComponentsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::BuildStampComponents");

        world.component::<StampShown>();
        world
            .component::<BuildStamp>()
            .add_trait::<flecs::Singleton>();
        world.set(BuildStamp::default());
    }
}

/// Behaviour: keep each player's build bar equal to what the stamp currently
/// says.
#[derive(Component)]
pub struct BuildStampModule;

impl Module for BuildStampModule {
    fn module(world: &World) {
        world.module::<Self>("smash::BuildStamp");
        world.import::<BuildStampComponentsModule>();

        // A system rather than an `OnAdd` observer on `Player`, for the same
        // reason `smash::draw_projectiles` is one: a player is assembled over
        // several `set` calls and an observer on the first of them sees an
        // entity that has no `PlayerId` yet. Matching on what is needed and
        // skipping what is done sees a whole player or none of one. It is also
        // the only shape that can notice the wording changing under it, which
        // an observer on the player could not.
        //
        // Declared after `HudModule`'s `update_hud` -- `SmashModule` imports
        // this module last -- so on the tick a player joins, the match bar's
        // push is queued first and reaches the client first. A client stacks
        // boss bars in the order it is told about them, so that is what puts
        // the percentage on top and this underneath it.
        world
            .system_named::<&PlayerId>("show_build_stamp")
            .with(Player::id())
            .each_iter(|it, row, id| {
                let world = it.world();
                let bar = world.get::<&BuildStamp>(|stamp| stamp_bar(stamp, now_epoch()));
                let text = bar.title.plain();

                let entity = it.entity(row);
                let unchanged = entity
                    .try_get::<&StampShown>(|shown| shown.text == text)
                    .unwrap_or(false);
                if unchanged {
                    return;
                }

                world.get::<&ServerHandle>(|server| {
                    server.set_boss_bar(*id, BarSlot::Build, bar.clone());
                });
                entity.set(StampShown { text });
            });
    }
}
