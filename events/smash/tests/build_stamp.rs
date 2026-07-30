//! The build stamp: what it says, and that it is said exactly once.
//!
//! Two halves, and the second one is the whole reason this file exists. The
//! wording is a pure function of a small struct and is pinned character for
//! character. The delivery is a whole world, run for hundreds of ticks through
//! every phase change a lobby has, asserting that the number of times a player
//! is told what build this is stays at one.
//!
//! That second claim is not a micro-optimisation. `hyperion::egress::boss_bar`
//! turns one `set_boss_bar` whose contents moved into one packet per viewer,
//! and a stamp that were pushed per tick would be twenty packets a second per
//! player carrying a string that cannot change until the process exits.

mod harness;

use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        build_stamp::{BuildStamp, stamp_bar, utc_minute},
        lobby::{Lobby, LobbyConfig, Phase},
    },
    server::{BarColour, BarSlot, NamedColor, PlayerId, TextColor, mock::Call},
};

/// A stamp for a clean build of one particular commit.
fn clean() -> BuildStamp {
    BuildStamp {
        rev: Some("8f3a21c".to_owned()),
        committed_at: Some(1_785_348_240),
        dirty: false,
    }
}

// ---------------------------------------------------------------------------
// what it says
// ---------------------------------------------------------------------------

/// The commit and the minute it was made, in that order, on one line.
#[test]
fn the_stamp_names_the_commit_and_when_it_was_made() {
    let bar = stamp_bar(&clean());
    assert_eq!(
        bar.title.plain(),
        "build 8f3a21c \u{b7} 2026-07-29 18:04 UTC"
    );
    assert_eq!(
        bar.title.runs()[0].color(),
        Some(TextColor::Named(NamedColor::Gray))
    );
    assert_eq!(bar.colour, BarColour::Blue);
    // Empty, so the strip draws no coloured length. A build is not a fraction
    // of anything and the fill would be inventing a quantity.
    assert!(bar.progress.abs() < 1e-9, "{}", bar.progress);
}

/// A dirty build says so, and says it in red.
///
/// The one case where the commit on screen is not the code that is running is
/// the one case a reader must not skim past, so it changes the colour of the
/// whole strip rather than adding a word to the end of a line.
#[test]
fn a_dirty_build_says_so_and_turns_the_bar_red() {
    let bar = stamp_bar(&BuildStamp {
        dirty: true,
        ..clean()
    });
    assert_eq!(
        bar.title.plain(),
        "build 8f3a21c + uncommitted changes \u{b7} 2026-07-29 18:04 UTC"
    );
    assert_eq!(
        bar.title.runs()[0].color(),
        Some(TextColor::Named(NamedColor::Red)),
        "a stamp that cannot be trusted has to look different"
    );
    assert_eq!(bar.colour, BarColour::Red);
}

/// A build nobody stamped says that, rather than a commit it does not have.
///
/// This is the `cargo run` case, and the wording is aimed at the person who
/// will meet it: a developer on their own machine, who needs to know that the
/// blank is expected and not a broken deploy.
#[test]
fn an_unstamped_build_says_it_is_unpackaged() {
    let bar = stamp_bar(&BuildStamp::default());
    assert_eq!(bar.title.plain(), "build unpackaged build");
    assert_eq!(bar.colour, BarColour::Blue);
}

/// A rev with no time gets the rev alone, rather than a separator with nothing
/// after it.
#[test]
fn half_a_stamp_is_shown_rather_than_none_of_one() {
    let bar = stamp_bar(&BuildStamp {
        committed_at: None,
        ..clean()
    });
    assert_eq!(bar.title.plain(), "build 8f3a21c");
}

/// The environment is parsed the way the wrapper writes it, and a field that
/// cannot be used is dropped rather than shown as itself.
#[test]
fn a_field_that_is_not_usable_reads_as_absent() {
    assert_eq!(
        BuildStamp::parse(Some("8f3a21c"), Some("1785348240"), Some("1")),
        BuildStamp {
            rev: Some("8f3a21c".to_owned()),
            committed_at: Some(1_785_348_240),
            dirty: true,
        }
    );
    // An empty rev is what an unset variable looks like when something exports
    // it anyway, and a time that is not a number is what a broken wrapper
    // produces. Neither is worth putting on a screen.
    assert_eq!(
        BuildStamp::parse(Some("   "), Some("not-a-time"), Some("0")),
        BuildStamp::default()
    );
    assert_eq!(BuildStamp::parse(None, None, None), BuildStamp::default());
    // What the wrapper writes for a source with no git in it: both variables
    // set, both empty. The flake refuses to emit a time without a rev, because
    // `lastModified` on such a source is a directory mtime, and an empty
    // string has to read the same way here as an unset variable or the refusal
    // would only have moved the problem.
    assert_eq!(
        BuildStamp::parse(Some(""), Some(""), Some("0")),
        BuildStamp::default()
    );
    // Anything but exactly `1` is clean, so a wrapper that writes `false` or
    // `no` does not accidentally mark every build dirty.
    assert!(!BuildStamp::parse(None, None, Some("true")).dirty);
}

/// The date arithmetic, at the instants that break a wrong implementation.
///
/// A leap day, a century that is not a leap year, and a time before the epoch:
/// each of the three is a different way to get the arithmetic wrong, and none
/// of them is reachable by a test that only uses today's date.
#[test]
fn the_clock_is_utc_and_survives_the_awkward_dates() {
    assert_eq!(utc_minute(0), "1970-01-01 00:00 UTC");
    assert_eq!(utc_minute(1_000_000_000), "2001-09-09 01:46 UTC");
    assert_eq!(utc_minute(1_785_348_240), "2026-07-29 18:04 UTC");
    // 2000 is divisible by 400, so it has a 29th of February.
    assert_eq!(utc_minute(951_782_400), "2000-02-29 00:00 UTC");
    // 2100 is divisible by 100 and not by 400, so it does not.
    assert_eq!(utc_minute(4_107_542_400), "2100-03-01 00:00 UTC");
    // Before the epoch, which is where a truncating division goes wrong.
    assert_eq!(utc_minute(-1), "1969-12-31 23:59 UTC");
}

// ---------------------------------------------------------------------------
// that it reaches a player, once
// ---------------------------------------------------------------------------

/// Every stamp a player was sent, in order.
fn stamps_to(game: &Game, player: PlayerId) -> Vec<String> {
    game.server
        .boss_bars_of(player, BarSlot::Build)
        .iter()
        .map(|bar| bar.title.plain())
        .collect()
}

/// The stamp reaches the player, with the words the pure function produced.
#[test]
fn a_player_is_told_what_build_they_are_standing_in() {
    let mut game = Game::new();
    game.world.set(clean());
    game.player("p", Vec3::new(0.0, 100.0, 0.0));
    game.advance(0.05, 1);

    assert_eq!(stamps_to(&game, PlayerId(1)), vec![
        "build 8f3a21c \u{b7} 2026-07-29 18:04 UTC".to_owned()
    ]);
}

/// Once, and not once a tick.
///
/// Four hundred ticks with a lobby short enough to run a whole match inside
/// them, so the run crosses every phase change the game has and the match bar
/// beside this one is rewritten hundreds of times. The stamp is still one
/// call, because the tag the system adds is what stops the query matching.
#[test]
fn the_stamp_is_sent_once_and_not_once_a_tick() {
    let mut game = Game::new();
    game.world.set(clean());
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 1.0,
        countdown_at_three_quarters: 0.75,
        countdown_at_full: 0.5,
        prepare_seconds: 0.5,
        match_timeout_seconds: 5.0,
        results_seconds: 0.5,
    });
    game.player("a", Vec3::new(0.0, 100.0, 0.0));
    game.player("b", Vec3::new(4.0, 100.0, 0.0));

    game.advance(20.0, 400);

    // The run really did move the other bar, or "one stamp" would be a claim
    // about a world in which nothing happened at all.
    let match_bars = game.server.boss_bars_of(PlayerId(1), BarSlot::Hud).len();
    assert!(
        match_bars > 1,
        "the match bar never moved, so this proves nothing: {match_bars}"
    );
    assert_ne!(
        game.world.cloned::<&Lobby>().phase,
        Phase::Waiting,
        "the lobby never left Waiting, so no phase change was crossed"
    );

    for player in [PlayerId(1), PlayerId(2)] {
        let stamps = stamps_to(&game, player);
        // The count and one example, not the whole run: a bar resent every
        // tick produces four hundred identical strings, and a failure nobody
        // can read is one nobody acts on.
        assert_eq!(
            stamps.len(),
            1,
            "{player:?} was told the build {} times, saying {:?}",
            stamps.len(),
            stamps.first()
        );
    }
}

/// Somebody who arrives later gets it too.
///
/// The interesting half is that they get it *without* anybody else being told
/// again: a joiner is a new row in the system's query and not a reason to
/// redraw the players already standing there.
#[test]
fn a_late_joiner_gets_the_stamp_and_nobody_else_gets_it_twice() {
    let mut game = Game::new();
    game.world.set(clean());
    game.player("early", Vec3::new(0.0, 100.0, 0.0));
    game.advance(1.0, 20);
    assert_eq!(stamps_to(&game, PlayerId(1)).len(), 1);

    game.server.take();
    game.player("late", Vec3::new(4.0, 100.0, 0.0));
    game.advance(1.0, 20);

    assert_eq!(
        stamps_to(&game, PlayerId(2)).len(),
        1,
        "the late joiner was not told what build this is"
    );
    assert_eq!(
        stamps_to(&game, PlayerId(1)),
        Vec::<String>::new(),
        "somebody else joining re-sent the stamp to a player who had it"
    );
}

/// Every slot is in `BarSlot::ALL`, and its index is a place inside an array
/// of `BarSlot::COUNT`.
///
/// `adapter::PlayerBars` is `[Option<Entity>; BarSlot::COUNT]` and is written
/// at `slot.index()`, in a system in the server's `PostUpdate`. If those two
/// ever disagree the failure is an out-of-bounds panic on a live server the
/// first tick anybody writes the new slot, which is the worst place to find
/// out and is why this is pinned here rather than trusted.
#[test]
fn every_slot_is_in_all_and_indexes_into_it() {
    // Exhaustive on purpose. A new variant makes this match fail to compile,
    // so whoever adds one is sent here, and here is where `BarSlot::ALL` is
    // checked to have grown with it.
    let listed: Vec<BarSlot> = vec![BarSlot::Hud, BarSlot::Build]
        .into_iter()
        .inspect(|slot| match slot {
            BarSlot::Hud | BarSlot::Build => {}
        })
        .collect();
    assert_eq!(
        BarSlot::ALL,
        listed.as_slice(),
        "a slot is missing from BarSlot::ALL, so its index would panic"
    );

    assert_eq!(BarSlot::COUNT, BarSlot::ALL.len());
    for (at, slot) in BarSlot::ALL.iter().enumerate() {
        assert_eq!(slot.index(), at, "{slot:?} does not index its own position");
        assert!(
            slot.index() < BarSlot::COUNT,
            "{slot:?} indexes past a PlayerBars array"
        );
    }
}

/// The stamp goes to its own slot, so it can never overwrite the match bar.
///
/// Both bars are pushed on the tick a player joins. Without the slot the
/// second of the two would replace the first on the client, and which one
/// survived would depend on the order two systems happened to be declared in.
#[test]
fn the_stamp_and_the_match_bar_are_different_bars() {
    let mut game = Game::new();
    game.world.set(clean());
    game.player("p", Vec3::new(0.0, 100.0, 0.0));
    game.advance(0.05, 1);

    let slots: Vec<BarSlot> = game
        .server
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::BossBar(_, slot, _) => Some(slot),
            _ => None,
        })
        .collect();
    // The match bar first, so a client stacks the percentage above the stamp.
    assert_eq!(slots, vec![BarSlot::Hud, BarSlot::Build]);
}
