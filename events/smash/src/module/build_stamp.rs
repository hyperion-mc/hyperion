//! What build the server is running, on the player's screen.
//!
//! There is a live server that is redeployed as main moves, and until this
//! existed the only way to tell whether a change had reached it was to guess
//! at deploy timing from outside the game. The commit and the time it was made
//! are the two facts that answer it, so they go where a player is already
//! looking.
//!
//! # Why a second bar and not the lobby's
//!
//! The other candidate was [`crate::module::hud::boss_bar`]'s `Phase::Waiting`
//! arm -- the "Waiting for players 1/2" strip -- and it was rejected for two
//! reasons that point the same way.
//!
//! It is **not always there.** The bar becomes a countdown, then a percentage,
//! the moment a match starts, so a stamp folded into it answers the question
//! only for somebody who happens to arrive between matches. The question is
//! asked at arbitrary times, including by a person who joined a running match
//! to check, and half the time the lobby bar is a bar about something else.
//!
//! It **changes.** That title is a function of the player count, so a stamp
//! carried inside it is re-sent every time anybody joins or leaves -- as a
//! whole `Add`, because the title and the fill move together and
//! `hyperion::egress::boss_bar` collapses two moved fields into one. A build
//! stamp is constant for the life of a process and should cost exactly one
//! packet per viewer. Its own bar is the only shape that does.
//!
//! What that gives up is a permanent strip of screen in a fighting game, which
//! is a real cost. It is paid down as far as it goes: the bar is written once
//! per player and never again, its fill is left empty so it draws no coloured
//! length, and it is pushed after the match bar so it sits under it rather
//! than above.
//!
//! # Where the values come from
//!
//! Three environment variables, set by a wrapper the Nix build puts around the
//! server binary (see `flake.nix`). Not `env!` in a build script: baking a
//! commit hash into a crate makes every commit a full workspace recompile, and
//! the compile is already the floor on the pipeline. A wrapper is a symlink
//! and three assignments, so a new commit rebuilds that and nothing else.
//!
//! A `cargo run` build has none of them set and says so, rather than claiming
//! a commit it was not built from.

use std::env;

use flecs_ecs::prelude::*;

use crate::{
    module::player::Player,
    server::{BarColour, BarSlot, BossBar, NamedColor, PlayerId, ServerHandle, Text},
};

/// The short commit hash the server was built from, with no dirty marker on
/// it. That marker is [`DIRTY_VAR`]'s job.
pub const REV_VAR: &str = "HYPERION_BUILD_REV";

/// When that commit was made, in whole seconds since the unix epoch.
pub const TIME_VAR: &str = "HYPERION_BUILD_TIME";

/// `1` when the working tree had uncommitted changes at build time.
pub const DIRTY_VAR: &str = "HYPERION_BUILD_DIRTY";

/// What build this process is.
///
/// Every field is optional because the honest answer for a `cargo run` build
/// is that nobody said. A missing field reads as "unknown" on screen rather
/// than as a default that would be a lie -- a stamp that says `0000000` is
/// worse than one that says it does not know.
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
    /// What the environment says this build is.
    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(
            env::var(REV_VAR).ok().as_deref(),
            env::var(TIME_VAR).ok().as_deref(),
            env::var(DIRTY_VAR).ok().as_deref(),
        )
    }

    /// The same, from three strings.
    ///
    /// Split out from [`Self::from_env`] because `std::env::set_var` is unsafe
    /// in edition 2024 and racy in a test binary that runs its tests on
    /// threads. Everything worth pinning is in here, and it is reachable
    /// without touching the process environment at all.
    ///
    /// A field that is present but unusable -- an empty rev, a time that is
    /// not a number -- is treated as absent. The bar is a readout and half of
    /// one is better than none.
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
pub fn stamp_bar(stamp: &BuildStamp) -> BossBar {
    let rev = match (stamp.rev.as_deref(), stamp.dirty) {
        (Some(rev), true) => format!("{rev} + uncommitted changes"),
        (Some(rev), false) => rev.to_owned(),
        // Not "unknown" alone: the reason it is unknown is the useful half,
        // because it tells a developer looking at their own `cargo run` that
        // nothing is broken.
        (None, _) => "unpackaged build".to_owned(),
    };
    let title = stamp.committed_at.map_or_else(
        || format!("build {rev}"),
        |at| format!("build {rev} \u{b7} {}", utc_minute(at)),
    );
    BossBar {
        title: Text::text(title).color(if stamp.dirty {
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

/// `seconds` since the unix epoch as `YYYY-MM-DD HH:MM UTC`.
///
/// **Absolute and not "two hours ago".** A relative age is the friendlier
/// thing to read and it is the one thing this bar cannot say: it would change
/// every minute, and a bar that changes is a packet per viewer per change
/// forever, which is exactly the cost this whole design is built to avoid.
///
/// UTC and to the minute, because the reader is comparing it against a deploy
/// they watched and a commit timestamp they can read out of git, and both of
/// those are in UTC.
///
/// The date arithmetic is Howard Hinnant's `civil_from_days`, from
/// <https://howardhinnant.github.io/date_algorithms.html>, which is exact for
/// every year this will ever be handed and needs no table. There are no leap
/// seconds in unix time, so a day is 86,400 seconds and the split is a
/// division.
#[must_use]
pub fn utc_minute(seconds: i64) -> String {
    const SECONDS_PER_DAY: i64 = 86_400;

    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let within = seconds.rem_euclid(SECONDS_PER_DAY);
    let (hour, minute) = (within / 3600, (within % 3600) / 60);

    // Shift the epoch to 0000-03-01, which puts the leap day at the end of the
    // 400 year era and makes every month length a straight line.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// Marks a player who has already been sent the stamp, so it is sent once.
///
/// A tag and not a flag inside a record, because it is the query itself that
/// has to stop matching: once every connected player carries this, the system
/// below iterates nothing at all rather than iterating everybody to decide
/// there is nothing to do.
#[derive(Component, Debug)]
pub struct StampShown;

/// Registration: the types this file owns, and the stamp itself.
///
/// The environment is read here, once, where [`BuildStamp`] is registered. A
/// test that wants a particular build overwrites the singleton after importing
/// the game, which is why nothing else in this file ever looks at the
/// environment again.
#[derive(Component)]
pub struct BuildStampComponentsModule;

impl Module for BuildStampComponentsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::BuildStampComponents");

        world.component::<StampShown>();
        world
            .component::<BuildStamp>()
            .add_trait::<flecs::Singleton>();
        world.set(BuildStamp::from_env());
    }
}

/// Behaviour: put the stamp on each player's screen, exactly once.
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
        // skipping what is done sees a whole player or none of one.
        //
        // Declared after `HudModule`'s `update_hud` -- `SmashModule` imports
        // this module last -- so on the tick a player joins, the match bar's
        // push is queued first and reaches the client first. A client stacks
        // boss bars in the order it is told about them, so that is what puts
        // the percentage on top and this underneath it.
        world
            .system_named::<&PlayerId>("show_build_stamp")
            .with(Player::id())
            .without(StampShown::id())
            .each_iter(|it, row, id| {
                let world = it.world();
                let bar = world.get::<&BuildStamp>(stamp_bar);
                world.get::<&ServerHandle>(|server| {
                    server.set_boss_bar(*id, BarSlot::Build, bar.clone());
                });
                it.entity(row).add(StampShown::id());
            });
    }
}
