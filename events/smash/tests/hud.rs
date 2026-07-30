//! What the screen says, pinned exactly.
//!
//! Almost everything here drives a pure function, because almost everything in
//! `module/hud.rs` is one. That is deliberate and it is what makes this file
//! able to say anything: the interesting states of a heads-up display are the
//! boundaries -- the tick a cooldown finishes on, the second a countdown digit
//! appears, the percentage a bar turns red at, a match that ends with two
//! players level -- and a test that plays a match reaches those by luck if at
//! all.
//!
//! The last section drives a whole world instead, and it checks something the
//! pure functions cannot: that the numbers reach the seam, that they follow the
//! slot the player is actually holding, and that an unchanged screen sends
//! nothing.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        ability::{self, Cooldown, Grants, Slot},
        damage::{DamageKind, Damaged, hurt},
        hud::{
            METER_STEPS, RED_PERCENT, Recharge, View, YELLOW_PERCENT, boss_bar, countdown_title,
            death_title, meter, percent_colour, phase_title, winner_of,
        },
        kit,
        knockback::{Knockback, KnockbackModel, KnockbackTaken, percent, strength},
        lives::{DeathCause, Eliminated, Lives, kill},
        lobby::{Lobby, LobbyConfig, Phase},
        player::{Health, SelectedSlot},
    },
    server::{
        BarColour, BarSlot, Experience, NamedColor, PlayerId, TextColor, TitleTimes, mock::Call,
    },
};

const EPS: f32 = 1e-5;

/// The finest the experience bar and the boss bar are allowed to move, which is
/// therefore the tolerance every check on either of them gets.
fn step() -> f32 {
    1.0 / f32::from(METER_STEPS)
}

/// A view with every field at a value that makes no claim, so a test can set
/// only the fields its own claim is about.
const fn view() -> View<'static> {
    View {
        phase: Phase::Playing,
        players: 4,
        alive: 4,
        min_players: 4,
        timer: 0.0,
        span: 0.0,
        eliminated: false,
        percent: 0.0,
        health: 1.0,
        winner: None,
    }
}

// ---------------------------------------------------------------------------
// The experience bar
// ---------------------------------------------------------------------------

/// A ready ability is a full bar and no number.
///
/// Both halves. A full bar alone would be indistinguishable from a bar that
/// has finished filling but still carries a stale count beside it, which is
/// what a level nobody cleared would look like.
#[test]
fn a_ready_ability_fills_the_bar_and_shows_no_number() {
    let ready = meter(Some(Recharge {
        remaining: 0.0,
        full: 8.0,
    }));
    assert!((ready.progress - 1.0).abs() < EPS, "{ready:?}");
    assert_eq!(ready.level, 0, "{ready:?}");
}

/// An empty slot is an empty bar and no number, which is neither of the other
/// two states.
///
/// The state that has to exist and cannot be conflated: reporting "ready" for
/// a slot holding nothing would promise an ability that is not there, and
/// reporting "recharging" would promise one that is coming.
#[test]
fn an_empty_slot_is_an_empty_bar_and_no_number() {
    let empty = meter(None);
    assert_eq!(empty, Experience {
        progress: 0.0,
        level: 0
    });

    // And it is not the same value as either of the others, which is the whole
    // claim rather than a restatement of the line above.
    let ready = meter(Some(Recharge {
        remaining: 0.0,
        full: 8.0,
    }));
    let recharging = meter(Some(Recharge {
        remaining: 4.0,
        full: 8.0,
    }));
    assert_ne!(empty, ready);
    assert_ne!(empty, recharging);
    assert_ne!(ready, recharging);
}

/// The bar fills as the cooldown runs down, and the number counts the seconds.
#[test]
fn a_recharging_ability_fills_from_empty_and_counts_the_seconds() {
    // A ten second cooldown, sampled at each whole second left.
    let at = |remaining: f32| {
        meter(Some(Recharge {
            remaining,
            full: 10.0,
        }))
    };

    // Just after the press: nothing filled, ten seconds to wait.
    let fresh = at(10.0);
    assert!(fresh.progress < EPS, "{fresh:?}");
    assert_eq!(fresh.level, 10);

    // Half way.
    let half = at(5.0);
    assert!((half.progress - 0.5).abs() < step(), "{half:?}");
    assert_eq!(half.level, 5);

    // The bar only ever goes up and the number only ever goes down. Stepped in
    // whole ticks, counted as integers, so the sweep is the same on every
    // machine rather than depending on how a float accumulates.
    let mut previous = at(10.0);
    for tick in 1..=200_u16 {
        let remaining = f32::from(200 - tick) * 0.05;
        let now = at(remaining);
        assert!(
            now.progress >= previous.progress,
            "the bar went backwards at {remaining}s: {previous:?} then {now:?}"
        );
        assert!(
            now.level <= previous.level,
            "the number went up at {remaining}s: {previous:?} then {now:?}"
        );
        previous = now;
    }
}

/// A bar that is still recharging never reads full, at any point in any
/// cooldown the roster uses.
///
/// This is the property the quantisation exists to preserve and the one a
/// rounding to nearest would break: at the last tick of a ten second cooldown
/// the fraction left is 0.005, which rounds to a full bar and tells the player
/// they may press a button that will refuse them.
#[test]
fn a_recharging_bar_never_reads_full() {
    for cooldown in [1_u16, 5, 7, 8, 16, 30] {
        let full = f32::from(cooldown);
        // Every tick from the press to the last one that still refuses.
        for tick in 1..=cooldown * 20 {
            let remaining = f32::from(cooldown * 20 - tick + 1) * 0.05;
            let shown = meter(Some(Recharge { remaining, full }));
            assert!(
                shown.progress < 1.0,
                "a {full}s cooldown read as full with {remaining}s left"
            );
            assert!(
                shown.level >= 1,
                "a {full}s cooldown showed no number with {remaining}s left"
            );
        }
    }
}

/// An ability whose cooldown is zero is always ready, and never divides by it.
#[test]
fn a_zero_length_cooldown_is_always_ready() {
    for remaining in [0.0_f32, 1.0, f32::NAN] {
        let shown = meter(Some(Recharge {
            remaining,
            full: 0.0,
        }));
        assert!(
            (shown.progress - 1.0).abs() < EPS,
            "{remaining} -> {shown:?}"
        );
        assert_eq!(shown.level, 0);
    }
}

/// The bar is quantised, so a cooldown costs steps rather than ticks.
///
/// The number is the point of the quantisation and a test that only checked
/// monotonicity would pass at one packet per tick.
#[test]
fn a_cooldown_sends_at_most_one_packet_per_step() {
    const TICKS: u16 = 200;
    let mut seen: Vec<f32> = Vec::new();
    for tick in 0..TICKS {
        let shown = meter(Some(Recharge {
            remaining: f32::from(TICKS - tick) * 0.05,
            full: 10.0,
        }));
        if seen.last() != Some(&shown.progress) {
            seen.push(shown.progress);
        }
    }
    // Two hundred ticks in a ten second cooldown, and the bar takes at most as
    // many distinct values as it has steps, which is what turns two hundred
    // packets into sixty-four.
    let steps = usize::from(METER_STEPS);
    assert!(
        seen.len() <= steps,
        "a ten second cooldown produced {} distinct bars against {steps} steps",
        seen.len()
    );
    assert!(
        seen.len() > usize::from(TICKS) / 8,
        "the bar barely moved: {} values",
        seen.len()
    );
}

// ---------------------------------------------------------------------------
// The percentage
// ---------------------------------------------------------------------------

/// The percentage really is the knockback multiplier, minus one.
///
/// Not a restatement of `percent`'s own arithmetic: it solves the claim against
/// [`strength`], which is the function the whole game is balanced on. A hit on
/// a player at p% has to be `1 + p/100` times the same hit on a fresh one, and
/// if either function's health term is changed without the other this is what
/// fails.
#[test]
fn a_percentage_is_how_much_further_the_next_hit_sends_you() {
    let model = KnockbackModel::default();
    let full = Health::full(20.0);
    let baseline = strength(model, 6.0, full, KnockbackTaken::default(), 1.0);
    assert!(
        (percent(model, full)).abs() < EPS,
        "a fresh player is at 0%"
    );

    for current in [18.0_f32, 15.0, 10.0, 5.0, 1.0, 0.0] {
        let hurt = Health { current, max: 20.0 };
        let p = percent(model, hurt);
        let launched = strength(model, 6.0, hurt, KnockbackTaken::default(), 1.0);
        let expected = baseline * (1.0 + p / 100.0);
        assert!(
            (launched - expected).abs() < 1e-4,
            "at {current} health the percentage says {p}% but the model launched {launched} \
             against {expected}"
        );
    }

    // The end of the scale, for a twenty health kit.
    assert!(
        (percent(model, Health {
            current: 0.0,
            max: 20.0
        }) - 200.0)
            .abs()
            < EPS
    );
    // Overhealed is not a negative percentage.
    assert!(
        percent(model, Health {
            current: 30.0,
            max: 20.0
        })
        .abs()
            < EPS
    );
}

/// The bar changes colour exactly at the multipliers the constants name.
#[test]
fn the_bar_reddens_at_the_multiplier_the_bands_name() {
    assert_eq!(percent_colour(0.0), BarColour::Green);
    assert_eq!(percent_colour(YELLOW_PERCENT - 0.01), BarColour::Green);
    assert_eq!(percent_colour(YELLOW_PERCENT), BarColour::Yellow);
    assert_eq!(percent_colour(RED_PERCENT - 0.01), BarColour::Yellow);
    assert_eq!(percent_colour(RED_PERCENT), BarColour::Red);
    assert_eq!(percent_colour(500.0), BarColour::Red);
}

// ---------------------------------------------------------------------------
// The bar across the top
// ---------------------------------------------------------------------------

/// The lobby's bar says how many more players are needed.
#[test]
fn the_lobby_bar_counts_the_players_it_is_waiting_for() {
    let bar = boss_bar(&View {
        phase: Phase::Waiting,
        players: 3,
        min_players: 4,
        ..view()
    });
    assert_eq!(bar.title.plain(), "Waiting for players  3/4");
    // Three quarters is a whole number of steps, so quantising leaves it exact.
    assert!((bar.progress - 0.75).abs() < EPS, "{}", bar.progress);
    assert_eq!(bar.colour, BarColour::Blue);

    // An empty lobby is an empty bar rather than a division by zero.
    let none = boss_bar(&View {
        phase: Phase::Waiting,
        players: 0,
        min_players: 0,
        ..view()
    });
    assert!(none.progress.is_finite() && none.progress.abs() < EPS);
}

/// The countdown's bar drains, and says the seconds out loud.
#[test]
fn the_countdown_bar_drains_towards_the_start() {
    let bar = boss_bar(&View {
        phase: Phase::Countdown,
        timer: 7.5,
        span: 10.0,
        ..view()
    });
    assert_eq!(bar.title.plain(), "Starting in 8s");
    assert!((bar.progress - 0.75).abs() < EPS, "{}", bar.progress);
    assert_eq!(bar.colour, BarColour::Yellow);

    // Rounded up, so the bar never says zero while there is still a wait.
    let last = boss_bar(&View {
        phase: Phase::Countdown,
        timer: 0.05,
        span: 10.0,
        ..view()
    });
    assert_eq!(last.title.plain(), "Starting in 1s");
}

/// During a match the bar is the player's own percentage, and the bar under it
/// is their health.
#[test]
fn the_match_bar_is_the_percentage_and_drains_with_health() {
    let bar = boss_bar(&View {
        phase: Phase::Playing,
        percent: 140.0,
        health: 0.3,
        ..view()
    });
    assert_eq!(bar.title.plain(), "140%");
    // Within a step, because the bar is quantised: see `quantise`.
    assert!((bar.progress - 0.3).abs() <= step(), "{}", bar.progress);
    assert_eq!(bar.colour, BarColour::Red);
    assert_eq!(
        bar.title.runs()[0].color(),
        Some(TextColor::Named(NamedColor::Red)),
        "the number and the bar have to agree"
    );

    let fresh = boss_bar(&View {
        phase: Phase::Playing,
        percent: 0.0,
        health: 1.0,
        ..view()
    });
    assert_eq!(fresh.title.plain(), "0%");
    assert_eq!(fresh.colour, BarColour::Green);
}

/// A player who is out watches the match instead of their own body.
///
/// Their percentage is a fact about a corpse: health is zero, so it reads at
/// the top of the scale for the rest of the match and would be the loudest
/// thing on the screen of the one person it cannot happen to.
#[test]
fn a_spectator_sees_how_much_of_the_match_is_left() {
    let bar = boss_bar(&View {
        phase: Phase::Playing,
        eliminated: true,
        players: 8,
        alive: 2,
        percent: 200.0,
        health: 0.0,
        ..view()
    });
    assert_eq!(bar.title.plain(), "2 still in");
    assert!((bar.progress - 0.25).abs() < EPS, "{}", bar.progress);
    assert_eq!(bar.colour, BarColour::Blue);
}

/// The results bar names the winner, and says so when there is not one.
#[test]
fn the_results_bar_names_the_winner() {
    let won = boss_bar(&View {
        phase: Phase::Ended,
        winner: Some("Andrew"),
        ..view()
    });
    assert_eq!(won.title.plain(), "Andrew wins!");
    assert_eq!(won.colour, BarColour::Green);

    let drawn = boss_bar(&View {
        phase: Phase::Ended,
        winner: None,
        ..view()
    });
    assert_eq!(drawn.title.plain(), "Nobody was left standing");
    assert_eq!(drawn.colour, BarColour::Blue);
}

/// Every phase puts something on the bar, and no two phases put the same thing.
///
/// The claim the module documents is that the strip is never blank and never
/// stale. A per-phase test proves the first; this proves the second, which is
/// what a copy-pasted arm would break.
#[test]
fn every_phase_says_something_and_no_two_say_the_same() {
    let phases = [
        Phase::Waiting,
        Phase::Countdown,
        Phase::Preparing,
        Phase::Playing,
        Phase::Ended,
    ];
    let mut titles = Vec::new();
    for phase in phases {
        let bar = boss_bar(&View {
            phase,
            timer: 5.0,
            span: 10.0,
            ..view()
        });
        assert!(!bar.title.plain().is_empty(), "{phase:?} said nothing");
        assert!(
            (0.0..=1.0).contains(&bar.progress),
            "{phase:?} asked for a bar at {}",
            bar.progress
        );
        titles.push(bar.title.plain());
    }
    let mut unique = titles.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        titles.len(),
        "two phases read alike: {titles:?}"
    );
}

// ---------------------------------------------------------------------------
// Titles
// ---------------------------------------------------------------------------

/// Only the last three seconds get a number, and each is its own digit.
#[test]
fn only_the_last_three_seconds_of_the_countdown_get_a_number() {
    assert!(countdown_title(Phase::Preparing, 5.0).is_none());
    assert!(countdown_title(Phase::Preparing, 4.0).is_none());
    assert!(countdown_title(Phase::Preparing, 3.0001).is_none());
    assert!(countdown_title(Phase::Preparing, 0.0).is_none());
    assert!(countdown_title(Phase::Preparing, -1.0).is_none());

    for second in [3.0_f32, 2.0, 1.0] {
        let title = countdown_title(Phase::Preparing, second).expect("inside the window");
        assert_eq!(title.title.plain(), format!("{second:.0}"));
        assert_eq!(
            title.subtitle.as_ref().map(smash::server::Component::plain),
            Some("Get ready".to_owned())
        );
        assert_eq!(
            title.times,
            TitleTimes::TICK,
            "a countdown digit has to be gone before the next one arrives"
        );
    }
}

/// The lobby's own countdown gets no digits, only the prepare timer does.
///
/// Two phases count down in a row and both tick out loud, so a window on both
/// draws "3, 2, 1" twice with a teleport in the middle. The wire gate found
/// exactly that before this rule existed.
#[test]
fn only_the_prepare_timer_gets_digits() {
    for second in [3.0_f32, 2.0, 1.0] {
        for phase in [
            Phase::Waiting,
            Phase::Countdown,
            Phase::Playing,
            Phase::Ended,
        ] {
            assert!(
                countdown_title(phase, second).is_none(),
                "{phase:?} put a {second} on screen"
            );
        }
        assert!(countdown_title(Phase::Preparing, second).is_some());
    }
}

/// The start and the end get a title. Nothing else does.
#[test]
fn the_start_and_the_end_get_a_title_and_nothing_else_does() {
    assert!(phase_title(Phase::Waiting, None).is_none());
    assert!(phase_title(Phase::Countdown, None).is_none());
    assert!(phase_title(Phase::Preparing, None).is_none());

    let go = phase_title(Phase::Playing, None).expect("the match starting is worth a word");
    assert_eq!(go.title.plain(), "GO!");
    assert_eq!(go.times, TitleTimes::TICK);

    let won = phase_title(Phase::Ended, Some("Andrew")).expect("a result is worth a title");
    assert_eq!(won.title.plain(), "Andrew wins!");
    assert_eq!(
        won.subtitle.as_ref().map(smash::server::Component::plain),
        Some("Last mob standing".to_owned())
    );

    let drawn = phase_title(Phase::Ended, None).expect("so is the absence of one");
    assert_eq!(drawn.title.plain(), "Game over");
    assert_eq!(
        drawn.subtitle.as_ref().map(smash::server::Component::plain),
        Some("Nobody was left standing".to_owned())
    );
}

/// A death says what it cost, and who did it.
#[test]
fn a_death_names_the_killer_under_the_life_count() {
    let title = death_title(2, Some("Andrew"), DeathCause::Damage);
    assert_eq!(title.title.plain(), "2 lives left!");
    assert_eq!(
        title.subtitle.as_ref().map(smash::server::Component::plain),
        Some("Smashed by Andrew".to_owned()),
        "the whole point of the subtitle"
    );

    // The last life, and the credit still arrives.
    let last = death_title(0, Some("Andrew"), DeathCause::Void);
    assert_eq!(last.title.plain(), "GAME OVER");
    assert_eq!(
        last.subtitle.as_ref().map(smash::server::Component::plain),
        Some("Smashed by Andrew".to_owned())
    );

    // Nobody to credit, and the two ways that happens read differently.
    let fell = death_title(1, None, DeathCause::Void);
    assert_eq!(
        fell.subtitle.as_ref().map(smash::server::Component::plain),
        Some("You fell out of the world".to_owned())
    );
    let starved = death_title(1, None, DeathCause::Damage);
    assert_eq!(
        starved
            .subtitle
            .as_ref()
            .map(smash::server::Component::plain),
        Some("Nobody to blame but yourself".to_owned())
    );
}

/// One life left is one life, not one lives.
///
/// A trivial line, and the reason it has a test is that it is read at the worst
/// moment of a match: a grammatical error there is exactly the kind of thing
/// that makes a finished game feel unfinished.
#[test]
fn one_life_left_is_not_pluralised() {
    assert_eq!(
        death_title(1, None, DeathCause::Void).title.plain(),
        "1 life left!"
    );
    assert_eq!(
        death_title(2, None, DeathCause::Void).title.plain(),
        "2 lives left!"
    );
    assert_eq!(
        death_title(3, None, DeathCause::Void).title.plain(),
        "3 lives left!"
    );
}

// ---------------------------------------------------------------------------
// Who won
// ---------------------------------------------------------------------------

/// The last player standing wins.
#[test]
fn the_last_player_standing_wins() {
    let standings = [
        ("out".to_owned(), 0u8),
        ("alive".to_owned(), 2u8),
        ("also_out".to_owned(), 0u8),
    ];
    assert_eq!(winner_of(&standings), Some("alive"));
}

/// A match that times out is won by whoever has the most lives.
#[test]
fn a_timed_out_match_goes_to_the_most_lives() {
    let standings = [
        ("ahead".to_owned(), 4u8),
        ("behind".to_owned(), 2u8),
        ("out".to_owned(), 0u8),
    ];
    assert_eq!(winner_of(&standings), Some("ahead"));
}

/// Two players level is not a winner, and neither is nobody.
///
/// Picking one of them would be inventing a result the game does not have, and
/// a results screen that names a winner is what a player believes.
#[test]
fn a_tie_has_no_winner() {
    let level = [("one".to_owned(), 3u8), ("two".to_owned(), 3u8)];
    assert_eq!(winner_of(&level), None);

    let nobody = [("one".to_owned(), 0u8), ("two".to_owned(), 0u8)];
    assert_eq!(winner_of(&nobody), None);

    let empty: [(String, u8); 0] = [];
    assert_eq!(winner_of(&empty), None);
}

// ---------------------------------------------------------------------------
// In a world
// ---------------------------------------------------------------------------

/// Give `player` a kit and hold `slot`, returning the ability instance there.
fn hold(game: &Game, player: EntityView<'_>, kit_name: &str, slot: u8) -> Entity {
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, kit_name).expect("a stock kit"),
    );
    player.set(SelectedSlot(slot));
    let mut found = None;
    player.each_target(Grants, |ability| {
        if ability.try_get::<&Slot>(|s| s.0 == slot) == Some(true) {
            found = Some(ability.id());
        }
    });
    found.expect("the kit grants an ability in that slot")
}

/// The last experience bar the server pushed to a player.
fn bar_of(game: &Game, player: PlayerId) -> Option<Experience> {
    game.server.experience_of(player).last().copied()
}

/// The experience bar follows the cooldown of the slot being held.
///
/// The whole feature, end to end through the seam: full before the press, then
/// filling with a number beside it, then full again with the number gone, and
/// the ability's own cooldown component agreeing at each step.
#[test]
fn the_experience_bar_follows_the_cooldown_of_the_slot_being_held() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    // Slot 2 is Seismic Slam at seven seconds. Abilities take the slots in the
    // order the kit declares them, so this is the third `.ability` call in
    // `kits/iron_golem.rs` and the assertion below is what says so.
    let ability = hold(&game, player, "Iron Golem", 2);
    let cooldown = game
        .world
        .entity_from_id(ability)
        .cloned::<&smash::module::ability::CooldownSpec>()
        .0;
    assert!((cooldown - 7.0).abs() < EPS, "the kit changed: {cooldown}s");

    game.advance(0.05, 1);
    assert_eq!(
        bar_of(&game, PlayerId(1)),
        Some(Experience {
            progress: 1.0,
            level: 0
        }),
        "a ready ability is a full bar"
    );

    game.server.take();
    assert_eq!(ability::activate(player, 2, 1.0), Ok(()));
    game.advance(0.05, 1);
    let fired = bar_of(&game, PlayerId(1)).expect("the press moved the bar");
    assert!(fired.progress < 2.0 * step(), "{fired:?}");
    assert_eq!(fired.level, 7, "the number is the seconds left");

    // Two seconds in: five to go, and the bar is a bit over two sevenths.
    game.advance(2.0, 40);
    let part = bar_of(&game, PlayerId(1)).expect("the bar kept moving");
    assert_eq!(part.level, 5, "{part:?}");
    assert!((part.progress - 2.0 / 7.0).abs() < 2.0 * step(), "{part:?}");

    // Out the other side.
    game.advance(5.5, 110);
    assert!(
        game.world
            .entity_from_id(ability)
            .cloned::<&Cooldown>()
            .remaining
            .abs()
            < EPS,
        "the cooldown has not finished, so this test is measuring the wrong thing"
    );
    assert_eq!(
        bar_of(&game, PlayerId(1)),
        Some(Experience {
            progress: 1.0,
            level: 0
        }),
        "a finished cooldown is a full bar and no number again"
    );
}

/// Changing which slot is held changes which cooldown the bar shows.
///
/// The bar is not "the last ability you used", which is the shape a naive
/// implementation lands on and which reads identically until a player holds a
/// different item.
#[test]
fn changing_the_held_slot_changes_which_cooldown_the_bar_shows() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    hold(&game, player, "Iron Golem", 2);
    player.set(smash::module::player::OnGround(true));

    assert_eq!(ability::activate(player, 2, 1.0), Ok(()));
    game.advance(0.05, 1);
    let firing = bar_of(&game, PlayerId(1)).expect("a bar");
    assert!(firing.level > 0, "the fired slot is recharging: {firing:?}");

    // Slot 1 is Iron Hook, untouched.
    player.set(SelectedSlot(1));
    game.advance(0.05, 1);
    assert_eq!(
        bar_of(&game, PlayerId(1)),
        Some(Experience {
            progress: 1.0,
            level: 0
        }),
        "holding an untouched slot shows that slot"
    );

    // Slot 5 holds nothing at all.
    player.set(SelectedSlot(5));
    game.advance(0.05, 1);
    assert_eq!(
        bar_of(&game, PlayerId(1)),
        Some(Experience {
            progress: 0.0,
            level: 0
        }),
        "holding an empty slot is an empty bar"
    );
}

/// An unchanged screen sends nothing.
///
/// Both surfaces are a packet per player and the cheapest possible bug here is
/// a redraw every tick, which costs twenty packets a second per player and is
/// invisible in play. The lobby is the state that changes least, so it is where
/// the claim is testable.
#[test]
fn an_unchanged_screen_sends_nothing() {
    let mut game = Game::new();
    game.player("p", Vec3::new(0.0, 100.0, 0.0));
    // One player against a minimum of four, so the lobby stays in Waiting and
    // nothing else moves.
    game.advance(1.0, 20);

    let hud_calls = |game: &Game| {
        game.server
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::Experience(..) | Call::BossBar(..)))
            .count()
    };
    // Two, not three. A player with no kit holds an empty slot, whose meter is
    // the empty bar a fresh client already draws, so there is nothing to
    // correct. The two bars are the only things the client does not already
    // have: the match bar, and the build stamp under it.
    assert_eq!(
        hud_calls(&game),
        2,
        "a joining player with no kit needs two pushes: {:?}",
        game.server.calls()
    );
    assert_eq!(
        game.server.boss_bars_of(PlayerId(1), BarSlot::Build).len(),
        1,
        "the build stamp is one of them: {:?}",
        game.server.calls()
    );

    game.server.take();
    game.advance(5.0, 100);
    let after = hud_calls(&game);
    assert_eq!(after, 0, "a hundred idle ticks sent {after} HUD packets");
}

/// A hit moves the percentage on the victim's own bar.
#[test]
fn a_hit_moves_the_percentage_on_the_victims_bar() {
    let mut game = Game::new();
    // Well above the arena's kill plane, because setting the phase by hand
    // skips the scatter that would otherwise have put them on a platform.
    let attacker = game.player("attacker", Vec3::new(0.0, 100.0, 0.0));
    let victim = game.player("victim", Vec3::new(2.0, 100.0, 0.0));
    let victim_view = game.world.entity_from_id(victim);
    game.world.set(Lobby {
        phase: Phase::Playing,
        timer: 1.0,
    });
    game.advance(0.05, 1);

    let before = game
        .server
        .boss_bars_of(PlayerId(2), BarSlot::Hud)
        .last()
        .cloned()
        .expect("a bar during a match");
    assert_eq!(before.title.plain(), "0%");
    assert_eq!(before.colour, BarColour::Green);

    // Ten of twenty health, which the model says is exactly 100%.
    hurt(victim_view, Damaged {
        attacker: Some(attacker),
        amount: 10.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Environment,
    });
    game.advance(0.05, 1);

    let after = game
        .server
        .boss_bars_of(PlayerId(2), BarSlot::Hud)
        .last()
        .cloned()
        .expect("the hit moved the bar");
    assert_eq!(after.title.plain(), "100%");
    assert_eq!(after.colour, BarColour::Red);
    assert!((after.progress - 0.5).abs() < EPS, "{}", after.progress);
}

/// A death puts the killer's name on the victim's own screen.
///
/// The credit relation has existed since the damage pipeline was written and
/// until now it only ever reached the broadcast chat line, which is read by
/// everybody except the person it happened to.
#[test]
fn a_death_puts_the_killers_name_on_the_victims_screen() {
    let mut game = Game::new();
    let attacker = game.player("Andrew", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(2.0, 0.0, 0.0));
    let victim_view = game.world.entity_from_id(victim);

    hurt(victim_view, Damaged {
        attacker: Some(attacker),
        amount: 1.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });
    game.server.take();
    kill(victim_view, DeathCause::Void);

    let titles = game.server.titles_to(PlayerId(2));
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert_eq!(titles[0].title.plain(), "3 lives left!");
    assert_eq!(
        titles[0]
            .subtitle
            .as_ref()
            .map(smash::server::Component::plain),
        Some("Smashed by Andrew".to_owned())
    );
}

/// The end of a match names the winner on every screen and in the chat.
#[test]
fn the_end_of_a_match_names_the_winner() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 2,
        countdown_at_full: 0.1,
        countdown_at_three_quarters: 0.1,
        countdown_at_min: 0.1,
        prepare_seconds: 0.1,
        match_timeout_seconds: 60.0,
        results_seconds: 5.0,
    });
    let doomed = game.player("doomed", Vec3::ZERO);
    game.player("survivor", Vec3::new(30.0, 0.0, 0.0));
    let doomed = game.world.entity_from_id(doomed);

    // Into the match.
    game.advance(1.0, 20);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);

    // Out of lives, which is what ends it.
    for _ in 0..smash::module::lives::MAX_LIVES {
        kill(doomed, DeathCause::Void);
        doomed.remove(smash::module::lives::RespawnAt::id());
        doomed.get::<&mut Health>(|health| health.current = health.max);
    }
    assert!(doomed.has(Eliminated::id()));

    game.server.take();
    game.advance(0.5, 10);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Ended);

    let titles = game.server.broadcast_titles();
    assert!(
        titles
            .iter()
            .any(|title| title.title.plain() == "survivor wins!"),
        "{titles:?}"
    );
    assert!(
        game.server
            .broadcasts()
            .iter()
            .any(|line| line == "Game over. survivor wins!"),
        "{:?}",
        game.server.broadcasts()
    );
}

/// A player still in the match is never told somebody else's bar.
///
/// One system pushes every player's bar in one pass, so the failure worth
/// guarding against is a loop that computes one view and sends it to everybody.
#[test]
fn each_player_gets_their_own_percentage() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::new(0.0, 100.0, 0.0));
    let victim = game.player("victim", Vec3::new(2.0, 100.0, 0.0));
    game.world.set(Lobby {
        phase: Phase::Playing,
        timer: 1.0,
    });
    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: Some(attacker),
        amount: 10.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Environment,
    });
    game.advance(0.05, 2);

    let hit = game.server.boss_bars_of(PlayerId(2), BarSlot::Hud);
    let untouched = game.server.boss_bars_of(PlayerId(1), BarSlot::Hud);
    assert_eq!(
        hit.last().map(|bar| bar.title.plain()),
        Some("100%".to_owned())
    );
    assert_eq!(
        untouched.last().map(|bar| bar.title.plain()),
        Some("0%".to_owned())
    );
}

/// The countdown's last three seconds reach every screen, exactly once.
///
/// Both counting phases are run through, which is the point: a four second
/// lobby countdown followed by a four second prepare timer crosses three, two
/// and one twice, and only the second crossing is a countdown to something a
/// player can act on.
#[test]
fn the_countdown_puts_a_number_on_every_screen_exactly_once() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 2,
        countdown_at_full: 4.0,
        countdown_at_three_quarters: 4.0,
        countdown_at_min: 4.0,
        prepare_seconds: 4.0,
        match_timeout_seconds: 60.0,
        results_seconds: 1.0,
    });
    game.player("one", Vec3::new(0.0, 100.0, 0.0));
    game.player("two", Vec3::new(4.0, 100.0, 0.0));

    // Through the lobby countdown and the prepare timer both.
    game.advance(9.0, 180);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);

    let shown: Vec<String> = game
        .server
        .broadcast_titles()
        .into_iter()
        .map(|title| title.title.plain())
        .collect();
    let digits: Vec<&String> = shown.iter().filter(|text| text.len() == 1).collect();
    assert_eq!(digits, vec!["3", "2", "1"], "{shown:?}");
    assert!(shown.contains(&"GO!".to_owned()), "{shown:?}");
}

/// Every player in the world is on the panel and the bar, including one who
/// joins mid-match.
///
/// `Shown` arrives with the player through a `With` trait, so a player created
/// after the module has been running still gets a first push rather than
/// silently having no record and never being compared.
#[test]
fn a_player_who_joins_late_gets_a_screen() {
    let mut game = Game::new();
    game.player("early", Vec3::new(0.0, 100.0, 0.0));
    game.advance(1.0, 20);

    let late = game.player("late", Vec3::new(4.0, 100.0, 0.0));
    let late = game.world.entity_from_id(late);
    hold(&game, late, "Iron Golem", 2);
    game.server.take();
    game.advance(0.05, 1);

    assert!(
        !game
            .server
            .boss_bars_of(PlayerId(2), BarSlot::Hud)
            .is_empty(),
        "the late joiner got no bar"
    );
    assert_eq!(
        game.server.experience_of(PlayerId(2)).last().copied(),
        Some(Experience {
            progress: 1.0,
            level: 0
        }),
        "the late joiner got no experience bar"
    );
    // And the early one is not resent anything by somebody else's arrival: the
    // lobby's own bar changed when the count went from one to two, so the check
    // is that nothing beyond that one push happened.
    assert_eq!(
        game.server.experience_of(PlayerId(1)).len(),
        0,
        "the early player's experience bar was rewritten for somebody else's arrival"
    );
}

/// Lives and elimination are not what the bar reads during a match, and a
/// spectator is not a live player with a strange percentage.
#[test]
fn elimination_switches_the_bar_to_spectating() {
    let mut game = Game::new();
    let doomed = game.player("doomed", Vec3::new(0.0, 100.0, 0.0));
    // Two survivors and not one, so the match does not end on the elimination
    // and the bar this test is about is still the one on screen.
    game.player("second", Vec3::new(30.0, 100.0, 0.0));
    game.player("third", Vec3::new(60.0, 100.0, 0.0));
    let doomed = game.world.entity_from_id(doomed);
    game.world.set(Lobby {
        phase: Phase::Playing,
        timer: 1.0,
    });

    for _ in 0..smash::module::lives::MAX_LIVES {
        kill(doomed, DeathCause::Void);
        doomed.remove(smash::module::lives::RespawnAt::id());
        doomed.get::<&mut Health>(|health| health.current = health.max);
    }
    assert_eq!(doomed.cloned::<&Lives>().0, 0);
    assert!(doomed.has(Eliminated::id()));
    game.advance(0.05, 2);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);

    let bar = game
        .server
        .boss_bars_of(PlayerId(1), BarSlot::Hud)
        .last()
        .cloned()
        .expect("a bar");
    assert_eq!(bar.title.plain(), "2 still in");
    assert!(
        (bar.progress - 2.0 / 3.0).abs() <= step(),
        "{}",
        bar.progress
    );
    assert_eq!(bar.colour, BarColour::Blue);
}

// ---------------------------------------------------------------------------
// In a deferred world, which is the only kind the running server has
// ---------------------------------------------------------------------------

/// The bar catches up with a kit chosen inside a deferred world.
///
/// Every path a player actually reaches runs inside a flecs system, so the
/// world is deferred and a `(Grants, ability)` edge written there is *queued*:
/// a reader in the same frame sees the state before the change. A test that
/// calls `choose` directly defers nothing and cannot see that window at all,
/// which is how a sibling change shipped a sound that played the previous kit's
/// voice with a green suite behind it.
///
/// What saves this module from the same bug is not luck but the shape of it:
/// the meter is recomputed and diffed every tick, so a queued edge costs one
/// tick of staleness and the next tick corrects it. That is a claim worth
/// pinning rather than assuming, because it is what makes a stale read
/// harmless here and fatal for a one-shot event.
#[test]
fn a_kit_chosen_in_a_deferred_world_reaches_the_bar() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::new(0.0, 100.0, 0.0));
    let player = game.world.entity_from_id(player);
    player.set(SelectedSlot(2));
    game.advance(0.05, 1);
    game.server.take();

    // Exactly what `/kit` does: `lobby::choose` from inside a system, where
    // every add it makes is queued rather than applied.
    game.world.defer(|| {
        smash::module::lobby::choose(
            &game.world,
            player,
            kit::by_name(&game.world, "Iron Golem").expect("a stock kit"),
        )
        .expect("the hub allows a kit change");
    });

    game.advance(0.05, 2);
    assert_eq!(
        bar_of(&game, PlayerId(1)),
        Some(Experience {
            progress: 1.0,
            level: 0
        }),
        "the bar never noticed the kit: {:?}",
        game.server.experience_of(PlayerId(1))
    );

    // And firing it in a deferred world moves the bar the same way.
    game.server.take();
    game.world.defer(|| {
        assert_eq!(ability::activate(player, 2, 1.0), Ok(()));
    });
    game.advance(0.05, 2);
    let fired = bar_of(&game, PlayerId(1)).expect("the press moved the bar");
    assert_eq!(fired.level, 7, "{fired:?}");
    assert!(fired.progress < 0.1, "{fired:?}");
}

/// A death in a deferred world still names the killer.
///
/// The credit is a relation the damage pipeline writes, so it is subject to the
/// same queueing. The hit and the death it causes are always separate ticks in
/// this game -- knockback carries a victim off the map over dozens of them --
/// so the edge has merged by the time the death reads it, and this drives the
/// two through separate deferred windows to say so rather than assuming it.
#[test]
fn a_death_in_a_deferred_world_still_names_the_killer() {
    let mut game = Game::new();
    let attacker = game.player("Andrew", Vec3::new(0.0, 100.0, 0.0));
    let victim = game.player("victim", Vec3::new(2.0, 100.0, 0.0));
    let victim_view = game.world.entity_from_id(victim);

    game.world.defer(|| {
        hurt(victim_view, Damaged {
            attacker: Some(attacker),
            amount: 1.0,
            knockback: Knockback::from(Vec3::ZERO),
            kind: DamageKind::Melee,
        });
    });
    game.advance(0.05, 1);
    game.server.take();

    game.world.defer(|| kill(victim_view, DeathCause::Void));

    let titles = game.server.titles_to(PlayerId(2));
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert_eq!(
        titles[0]
            .subtitle
            .as_ref()
            .map(smash::server::Component::plain),
        Some("Smashed by Andrew".to_owned()),
        "the credit did not survive the merge"
    );
}

/// The whole roster's cooldowns are readable on the bar.
///
/// Enumerated rather than sampled, because the failure this catches is one
/// ability quietly having no cooldown declared: it would read as permanently
/// ready and nothing else in the suite would notice, since `tests/abilities.rs`
/// checks the refusal rather than the display.
#[test]
fn every_ability_in_the_roster_has_a_readable_meter() {
    let game = Game::new();
    let mut checked = 0;
    for declared in ability::manifest(&game.world) {
        let fresh = meter(Some(Recharge {
            remaining: declared.cooldown,
            full: declared.cooldown,
        }));
        let ready = meter(Some(Recharge {
            remaining: 0.0,
            full: declared.cooldown,
        }));
        assert!(
            (ready.progress - 1.0).abs() < EPS && ready.level == 0,
            "{} / {} does not read as ready",
            declared.kit,
            declared.name
        );
        if declared.cooldown > 0.0 {
            assert!(
                fresh.progress < 1.0 && fresh.level >= 1,
                "{} / {} has a {}s cooldown that reads as ready the moment it fires",
                declared.kit,
                declared.name,
                declared.cooldown
            );
        } else {
            assert_eq!(
                fresh, ready,
                "{} / {} has no cooldown and should always read ready",
                declared.kit, declared.name
            );
        }
        checked += 1;
    }
    assert!(checked >= 50, "only {checked} abilities were enumerated");
}
