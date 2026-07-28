//! The lobby state machine, driven directly.

mod harness;

use smash::module::lobby::{Lobby, LobbyConfig, Phase, step};

fn config() -> LobbyConfig {
    LobbyConfig::default()
}

/// The player count one short of starting a countdown.
const fn below_min(config: &LobbyConfig) -> u32 {
    config.min_players - 1
}

/// The smallest count that satisfies `countdown_at_three_quarters`, which
/// `countdown_for` spells as `players * 4 >= full_players * 3`.
const fn three_quarters(config: &LobbyConfig) -> u32 {
    (config.full_players * 3).div_ceil(4)
}

// These name counts by what they mean to the config rather than by value. The
// literals 3, 4, 6 and 8 used to appear here, which stated `LobbyConfig`'s
// defaults a second time, so changing 4/8 to 2/4 broke six tests that were not
// about those numbers.

/// Run the machine until the phase changes or `limit` seconds pass.
fn run_until_change(
    config: &LobbyConfig,
    mut state: Lobby,
    players: u32,
    alive: u32,
) -> (Lobby, f32) {
    const DT: f32 = 0.05;
    let start = state.phase;
    let mut elapsed = 0.0;
    while state.phase == start && elapsed < 3600.0 {
        state = step(config, state, DT, players, alive);
        elapsed += DT;
    }
    (state, elapsed)
}

#[test]
fn an_empty_lobby_stays_waiting() {
    let config = config();
    let state = step(&config, Lobby::default(), 1.0, 0, 0);
    assert_eq!(state.phase, Phase::Waiting);
}

#[test]
fn one_short_of_the_minimum_still_waits() {
    let config = config();
    let state = step(&config, Lobby::default(), 1.0, config.min_players - 1, 0);
    assert_eq!(state.phase, Phase::Waiting);
}

#[test]
fn the_countdown_length_depends_on_how_full_the_lobby_is() {
    let config = config();
    assert_eq!(config.countdown_for(below_min(&config)), None);
    assert_eq!(
        config.countdown_for(config.min_players),
        Some(config.countdown_at_min)
    );
    assert_eq!(
        config.countdown_for(three_quarters(&config)),
        Some(config.countdown_at_three_quarters)
    );
    assert_eq!(
        config.countdown_for(config.full_players),
        Some(config.countdown_at_full)
    );
    assert_eq!(
        config.countdown_for(config.full_players * 3),
        Some(config.countdown_at_full),
        "an over-full lobby is still a full lobby"
    );
}

#[test]
fn reaching_the_minimum_starts_the_long_countdown() {
    let config = config();
    let at_min = config.min_players;
    let state = step(&config, Lobby::default(), 0.05, at_min, at_min);
    assert_eq!(state.phase, Phase::Countdown);
    assert!((state.timer - config.countdown_at_min).abs() < 1e-3);
}

#[test]
fn a_join_that_fills_the_lobby_shortens_the_countdown_immediately() {
    let config = config();
    let (at_min, full) = (config.min_players, config.full_players);
    let state = step(&config, Lobby::default(), 0.05, at_min, at_min);
    assert!(state.timer > config.countdown_at_three_quarters);

    let state = step(&config, state, 0.05, full, full);
    assert_eq!(state.phase, Phase::Countdown);
    assert!(
        state.timer <= config.countdown_at_full,
        "timer {} should have snapped down to {}",
        state.timer,
        config.countdown_at_full
    );
}

#[test]
fn the_countdown_never_lengthens_when_someone_leaves_but_stays_above_minimum() {
    let config = config();
    let (count, fewer) = (config.full_players, three_quarters(&config));
    let full = step(&config, Lobby::default(), 0.05, count, count);
    let after_leaver = step(&config, full, 0.05, fewer, fewer);
    assert!(after_leaver.timer <= full.timer);
}

#[test]
fn dropping_below_the_minimum_cancels_the_countdown_outright() {
    let config = config();
    let (at_min, short) = (config.min_players, below_min(&config));
    let state = step(&config, Lobby::default(), 0.05, at_min, at_min);
    assert_eq!(state.phase, Phase::Countdown);

    let state = step(&config, state, 0.05, short, short);
    assert_eq!(state.phase, Phase::Waiting);
    assert!(
        state.timer.abs() < 1e-6,
        "the clock is discarded, not paused"
    );
}

#[test]
fn the_full_path_from_waiting_to_results_and_back() {
    let config = config();
    let full = config.full_players;
    let mut state = Lobby::default();

    state = step(&config, state, 0.05, full, full);
    assert_eq!(state.phase, Phase::Countdown);

    let (state_after, elapsed) = run_until_change(&config, state, full, full);
    state = state_after;
    assert_eq!(state.phase, Phase::Preparing);
    assert!(
        (elapsed - config.countdown_at_full).abs() < 0.2,
        "countdown ran {elapsed}s, expected {}",
        config.countdown_at_full
    );

    let (state_after, elapsed) = run_until_change(&config, state, full, full);
    state = state_after;
    assert_eq!(state.phase, Phase::Playing);
    assert!((elapsed - config.prepare_seconds).abs() < 0.2);

    // Everyone still alive, so the match keeps running.
    let mid = step(&config, state, 1.0, full, full);
    assert_eq!(mid.phase, Phase::Playing);
    assert!(mid.timer > state.timer, "the match clock counts up");

    // One left standing ends it.
    state = step(&config, mid, 0.05, full, 1);
    assert_eq!(state.phase, Phase::Ended);
    assert!((state.timer - config.results_seconds).abs() < 1e-3);

    let (state, _) = run_until_change(&config, state, full, 1);
    assert_eq!(state.phase, Phase::Waiting);
}

#[test]
fn a_match_that_never_resolves_times_out() {
    let config = config();
    let state = Lobby {
        phase: Phase::Playing,
        timer: config.match_timeout_seconds - 0.5,
    };
    let full = config.full_players;
    let state = step(&config, state, 1.0, full, full);
    assert_eq!(
        state.phase,
        Phase::Ended,
        "twenty minutes is the hard stop; there is no sudden death"
    );
}

#[test]
fn a_solo_lobby_that_somehow_starts_ends_at_once() {
    let config = config();
    let state = Lobby {
        phase: Phase::Playing,
        timer: 0.0,
    };
    assert_eq!(step(&config, state, 0.05, 1, 1).phase, Phase::Ended);
    assert_eq!(
        step(&config, state, 0.05, config.full_players, 0).phase,
        Phase::Ended
    );
}

#[test]
fn preparing_cannot_be_cancelled_by_a_leaver() {
    let config = config();
    let state = Lobby {
        phase: Phase::Preparing,
        timer: config.prepare_seconds,
    };
    let state = step(&config, state, 0.05, 0, 0);
    assert_eq!(
        state.phase,
        Phase::Preparing,
        "once committed the match starts regardless"
    );
}

#[test]
fn a_phase_change_reaches_an_observer_that_names_lobby() {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use flecs_ecs::prelude::*;
    use harness::Game;
    use smash::module::lobby::PhaseChanged;

    let mut game = Game::new();
    for index in 0..4 {
        game.player(&format!("p{index}"), glam::Vec3::ZERO);
    }

    // Two observers, one per candidate term. Only the term the emit in
    // `lobby.rs` names may fire, and `terrain.rs` hangs the map rotation and
    // the return to the hub off exactly this: an observer whose term is wrong
    // registers happily and is never called, so the pin is that one counter
    // moves and the other does not.
    let by_lobby = Arc::new(AtomicU32::new(0));
    let by_arena = Arc::new(AtomicU32::new(0));

    let counted = Arc::clone(&by_lobby);
    game.world
        .observer_named::<PhaseChanged, ()>("test::by_lobby")
        .with(Lobby::id())
        .each_iter(move |_, _, ()| {
            counted.fetch_add(1, Ordering::Relaxed);
        });

    let counted = Arc::clone(&by_arena);
    game.world
        .observer_named::<PhaseChanged, ()>("test::by_arena")
        .with(smash::module::arena::Arena::id())
        .each_iter(move |_, _, ()| {
            counted.fetch_add(1, Ordering::Relaxed);
        });

    // Four players is at or past `min_players`, so the first tick leaves
    // Waiting.
    game.advance(0.05, 1);

    assert_eq!(
        game.world.cloned::<&Lobby>().phase,
        Phase::Countdown,
        "four players is enough to start, so the countdown should have started"
    );
    assert_eq!(
        by_lobby.load(Ordering::Relaxed),
        1,
        "the phase change did not reach an observer whose term is Lobby"
    );
    assert_eq!(
        by_arena.load(Ordering::Relaxed),
        0,
        "an observer whose term is not the emitted id must never fire"
    );
}
