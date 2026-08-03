//! The build stamp: what it says, how often it says it, and that a reload
//! changes it.
//!
//! Three halves, and the middle one is the reason this file is as long as it
//! is. The wording is a pure function of a small struct and a clock, and is
//! pinned character for character. The *rate* is pinned too, because the bar
//! now says `2h ago` rather than a UTC minute and a relative age changes on its
//! own: `hyperion::egress::boss_bar` turns one `set_boss_bar` whose contents
//! moved into one packet per viewer, so an age that read to the second would be
//! a packet per player per second forever. And the delivery is a whole world,
//! run for hundreds of ticks through every phase change a lobby has.
//!
//! Where a test is about delivery rather than wording it uses a stamp with no
//! commit time, whose rendering is therefore constant. The alternative is
//! asserting a string that depends on how long ago 2026-07-29 was when the test
//! ran, which passes today and fails tomorrow.

mod harness;

use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        build_stamp::{BuildStamp, relative_age, stamp_bar},
        lobby::{Lobby, LobbyConfig, Phase},
    },
    server::{BarColour, BarSlot, NamedColor, PlayerId, TextColor, mock::Call},
};

/// A stamp for a clean build of one particular commit.
fn clean() -> BuildStamp {
    BuildStamp {
        rev: Some("8f3a21c".to_owned()),
        committed_at: Some(COMMITTED_AT),
        dirty: false,
    }
}

/// The same build with nothing said about when it was made, so its bar reads
/// the same string whenever the test happens to run.
fn timeless() -> BuildStamp {
    BuildStamp {
        committed_at: None,
        ..clean()
    }
}

const COMMITTED_AT: i64 = 1_785_348_240;

/// Two hours and five minutes after [`COMMITTED_AT`].
const TWO_HOURS_LATER: i64 = COMMITTED_AT + 2 * 3600 + 5 * 60;

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

// ---------------------------------------------------------------------------
// what it says
// ---------------------------------------------------------------------------

/// The commit and how long ago it was made, in that order, on one line.
#[test]
fn the_stamp_names_the_commit_and_how_old_it_is() {
    let bar = stamp_bar(&clean(), TWO_HOURS_LATER);
    assert_eq!(bar.title.plain(), "build 8f3a21c \u{b7} 2h ago");
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
    let bar = stamp_bar(
        &BuildStamp {
            dirty: true,
            ..clean()
        },
        TWO_HOURS_LATER,
    );
    assert_eq!(
        bar.title.plain(),
        "build 8f3a21c + uncommitted changes \u{b7} 2h ago"
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
/// This is the `cargo run` case -- no `--build-stamp` directory on the command
/// line -- and the wording is aimed at the person who will meet it: a developer
/// on their own machine, who needs to know that the blank is expected and not a
/// broken deploy.
#[test]
fn an_unstamped_build_says_it_is_unpackaged() {
    let bar = stamp_bar(&BuildStamp::default(), TWO_HOURS_LATER);
    assert_eq!(bar.title.plain(), "build unpackaged build");
    assert_eq!(bar.colour, BarColour::Blue);
}

/// A rev with no time gets the rev alone, rather than a separator with nothing
/// after it.
#[test]
fn half_a_stamp_is_shown_rather_than_none_of_one() {
    assert_eq!(
        stamp_bar(&timeless(), TWO_HOURS_LATER).title.plain(),
        "build 8f3a21c"
    );
}

/// The three files are parsed the way the deploy writes them, and a field that
/// cannot be used is dropped rather than shown as itself.
#[test]
fn a_field_that_is_not_usable_reads_as_absent() {
    assert_eq!(
        BuildStamp::parse(Some("8f3a21c"), Some("1785348240"), Some("1")),
        BuildStamp {
            rev: Some("8f3a21c".to_owned()),
            committed_at: Some(COMMITTED_AT),
            dirty: true,
        }
    );
    // `environment.etc` ends every file it writes with a newline, so the
    // trimming is not defensive -- it is the format.
    assert_eq!(
        BuildStamp::parse(Some("8f3a21c\n"), Some("1785348240\n"), Some("0\n")),
        clean()
    );
    // An empty rev is what an unset value looks like when something writes the
    // file anyway, and a time that is not a number is what a broken deploy
    // produces. Neither is worth putting on a screen.
    assert_eq!(
        BuildStamp::parse(Some("   "), Some("not-a-time"), Some("0")),
        BuildStamp::default()
    );
    assert_eq!(BuildStamp::parse(None, None, None), BuildStamp::default());
    // What the flake writes for a source with no git in it: both files present,
    // both empty. The flake refuses to emit a time without a rev, because
    // `lastModified` on such a source is a directory mtime, and an empty file
    // has to read the same way here as an absent one or the refusal would only
    // have moved the problem.
    assert_eq!(
        BuildStamp::parse(Some(""), Some(""), Some("0")),
        BuildStamp::default()
    );
    // Anything but exactly `1` is clean, so a deploy that writes `false` or
    // `no` does not accidentally mark every build dirty.
    assert!(!BuildStamp::parse(None, None, Some("true")).dirty);
}

// ---------------------------------------------------------------------------
// how often it says it
// ---------------------------------------------------------------------------

/// Each unit, and the second on either side of every boundary between them.
#[test]
fn the_age_reads_in_the_coarsest_unit_that_still_says_something() {
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT), "just now");
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT + 59), "just now");
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT + MINUTE), "1m ago");
    assert_eq!(
        relative_age(COMMITTED_AT, COMMITTED_AT + HOUR - 1),
        "59m ago"
    );
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT + HOUR), "1h ago");
    assert_eq!(
        relative_age(COMMITTED_AT, COMMITTED_AT + DAY - 1),
        "23h ago"
    );
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT + DAY), "1d ago");
    assert_eq!(
        relative_age(COMMITTED_AT, COMMITTED_AT + 90 * DAY),
        "90d ago"
    );
}

/// A host whose clock is behind the machine that made the commit reads the
/// build as being from the future. `in -3m` on a boss bar is a worse answer
/// than a slightly early "just now".
#[test]
fn a_build_from_the_future_reads_as_just_now() {
    assert_eq!(relative_age(COMMITTED_AT, COMMITTED_AT - DAY), "just now");
    assert_eq!(relative_age(COMMITTED_AT, i64::MIN), "just now");
}

/// The packet budget, as arithmetic anybody can check.
///
/// This is the claim `module::build_stamp`'s own documentation makes -- at most
/// 60 packets per viewer in the first hour, 23 more across the rest of the
/// first day, one a day after -- restated as the number of distinct strings the
/// wording takes, because `show_build_stamp` sends exactly when that string
/// changes. Sampled every second, which is finer than the finest unit, so a
/// wording that read to the second would fail this by a factor of sixty.
#[test]
fn the_wording_changes_at_most_sixty_times_in_the_first_hour() {
    let distinct = |from: i64, to: i64| {
        let mut seen: Vec<String> = Vec::new();
        for second in from..to {
            let rendered = relative_age(COMMITTED_AT, COMMITTED_AT + second);
            if seen.last() != Some(&rendered) {
                seen.push(rendered);
            }
        }
        seen.len()
    };

    // "just now", then one string per minute for the other fifty-nine.
    assert_eq!(distinct(0, HOUR), 60);
    // Twenty-three more for the rest of the day.
    assert_eq!(distinct(HOUR, DAY), 23);
    // And one for the whole of the second day.
    assert_eq!(distinct(DAY, 2 * DAY), 1);
}

// ---------------------------------------------------------------------------
// that it reaches a player
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
    game.world.set(timeless());
    game.player("p", Vec3::new(0.0, 100.0, 0.0));
    game.advance(0.05, 1);

    assert_eq!(stamps_to(&game, PlayerId(1)), vec![
        "build 8f3a21c".to_owned()
    ]);
}

/// The age is on the bar a real player receives, and not only in the pure
/// function's return value.
///
/// The age itself cannot be pinned here -- it is measured against the wall
/// clock, so the exact string depends on when the test runs -- but its shape
/// can, and its shape is what would be missing if the system rendered the
/// stamp without a clock.
#[test]
fn the_bar_a_player_receives_carries_an_age() {
    let mut game = Game::new();
    game.world.set(clean());
    game.player("p", Vec3::new(0.0, 100.0, 0.0));
    game.advance(0.05, 1);

    let stamps = stamps_to(&game, PlayerId(1));
    let [only] = stamps.as_slice() else {
        panic!("expected exactly one stamp, got {stamps:?}");
    };
    assert!(only.starts_with("build 8f3a21c \u{b7} "), "{only}");
    assert!(only.ends_with(" ago"), "{only}");
}

/// Once, and not once a tick.
///
/// Four hundred ticks with a lobby short enough to run a whole match inside
/// them, so the run crosses every phase change the game has and the match bar
/// beside this one is rewritten hundreds of times. The stamp is still one call,
/// because the system compares what it would send against what it sent.
#[test]
fn the_stamp_is_sent_once_and_not_once_a_tick() {
    let mut game = Game::new();
    game.world.set(timeless());
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
        // The count and one example, not the whole run: a bar resent every tick
        // produces four hundred identical strings, and a failure nobody can
        // read is one nobody acts on.
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
    game.world.set(timeless());
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

/// A reload changes the stamp under a standing player, and their bar follows.
///
/// This is the behaviour hot reload needs and the old once-per-player tag could
/// not express: `init_game` re-reads `--build-stamp` on every accepted reload
/// and writes this singleton, and every player standing there has to end up
/// reading the new build rather than the one they joined under. The same
/// comparison is what makes the age advance on its own, and there is no way to
/// exercise that here without controlling the wall clock -- so it is exercised
/// through the other thing that changes the rendered string.
#[test]
fn a_stamp_that_changes_reaches_the_players_already_standing_there() {
    let mut game = Game::new();
    game.world.set(timeless());
    game.player("a", Vec3::new(0.0, 100.0, 0.0));
    game.player("b", Vec3::new(4.0, 100.0, 0.0));
    game.advance(1.0, 20);
    game.server.take();

    game.world.set(BuildStamp {
        rev: Some("deadbee".to_owned()),
        ..timeless()
    });
    game.advance(1.0, 20);

    for player in [PlayerId(1), PlayerId(2)] {
        assert_eq!(
            stamps_to(&game, player),
            vec!["build deadbee".to_owned()],
            "{player:?} is still being told about the build they joined under"
        );
    }
}

/// Every slot is in `BarSlot::ALL`, and its index is a place inside an array of
/// `BarSlot::COUNT`.
///
/// `adapter::PlayerBars` is `[Option<Entity>; BarSlot::COUNT]` and is written at
/// `slot.index()`, in a system in the server's `PostUpdate`. If those two ever
/// disagree the failure is an out-of-bounds panic on a live server the first
/// tick anybody writes the new slot, which is the worst place to find out and is
/// why this is pinned here rather than trusted.
#[test]
fn every_slot_is_in_all_and_indexes_into_it() {
    // Exhaustive on purpose. A new variant makes this match fail to compile, so
    // whoever adds one is sent here, and here is where `BarSlot::ALL` is checked
    // to have grown with it.
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
/// Both bars are pushed on the tick a player joins. Without the slot the second
/// of the two would replace the first on the client, and which one survived
/// would depend on the order two systems happened to be declared in.
#[test]
fn the_stamp_and_the_match_bar_are_different_bars() {
    let mut game = Game::new();
    game.world.set(timeless());
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
