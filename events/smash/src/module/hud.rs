//! What the screen says while a match runs.
//!
//! Three surfaces, and one rule behind all of them: the game already knows
//! everything a player needs, and until now it only said so in words, after
//! the fact, on a channel that scrolls.
//!
//! * The **experience bar** is the recharge of whatever ability is in the slot
//!   you are holding. Cooldowns were already enforced and the only feedback
//!   was a line of red text *after* a press had been refused, which is the
//!   wrong end of the interaction: a gauge answers "can I" before you commit,
//!   and a refusal only answers "you could not" once you already have.
//! * The **bar across the top** is your knockback percentage during a match,
//!   and whatever the lobby is waiting on outside one. Percentage is the read
//!   Super Smash Mobs is played on and no client draws it: hearts say how much
//!   damage you have left, and percent says what the next hit will do with it.
//!   See [`crate::module::knockback::percent`].
//! * The **titles** punctuate the match. The countdown gets a number, the
//!   start gets a word, the end gets the winner's name, and a death gets the
//!   name of whoever did it.
//!
//! Everything that decides content is a pure function of a small struct, and
//! everything that decides *when* lives in a system or in the lobby's own
//! state machine. That split is what makes `tests/hud.rs` able to pin the
//! exact bar, number and wording for every state the game has, including the
//! ones a scripted match would have to be lucky to reach.
//!
//! # Nothing is sent twice
//!
//! Each of these is a packet per player, and the bar and the meter both change
//! continuously. [`Shown`] is what the client was last told, initialised to
//! the state a fresh client is already in -- an empty experience bar and no
//! boss bar -- so the first push is the first real change and every tick after
//! that sends nothing until the value moves. The experience bar is also
//! quantised to [`METER_STEPS`], which is what keeps a seven second cooldown
//! to sixty-four packets rather than a hundred and forty.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        ability::{self, Cooldown, CooldownSpec},
        knockback::{self, KnockbackModel},
        lives::{self, DeathCause, Eliminated, Lives},
        lobby::{self, Lobby, LobbyConfig, Phase},
        player::{Health, Player, SelectedSlot},
    },
    server::{
        BarColour, BossBar, Experience, NamedColor, PlayerId, ServerHandle, Text, Title, TitleTimes,
    },
};

/// How many steps the experience bar is quantised to.
///
/// The bar is 182 pixels wide, so a step is under three pixels and no player
/// can see the difference between one step and the tick that produced it. What
/// the quantisation buys is the other direction: a cooldown otherwise sends a
/// packet every tick for its whole length, and this cuts a seven second one
/// from a hundred and forty to at most sixty-four.
pub const METER_STEPS: u16 = 64;

/// The cooldown of one ability, as the experience bar needs it.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Recharge {
    /// Seconds still to wait.
    pub remaining: f32,
    /// Seconds the wait is in total.
    pub full: f32,
}

/// The experience bar for a player holding an ability in `recharge`.
///
/// Three states, told apart by the two numbers together rather than by either
/// alone:
///
/// | in the slot | bar | number |
/// | --- | --- | --- |
/// | an ability that is ready | full | none |
/// | an ability recharging | filling from empty | whole seconds left |
/// | nothing | empty | none |
///
/// **Filling, not draining.** Vanilla fills this bar left to right as
/// experience accumulates, so a full bar already reads as "you have the thing"
/// to anybody who has played Minecraft, and a player learns the mapping
/// without being told. Draining would put the bar at its fullest at the exact
/// moment the ability cannot be used, which inverts the one fact it is there
/// to carry.
///
/// **The number is the seconds, rounded up.** The bar is the glanceable half
/// and is deliberately imprecise; the number is for the other question, which
/// is not "am I ready" but "how long until I am", and that is what a player
/// counts down when they are deciding whether to take a fight now or back off
/// for two seconds. Rounded up rather than down so it never says zero while
/// the ability is still refusing.
#[must_use]
pub fn meter(recharge: Option<Recharge>) -> Experience {
    /// A slot with nothing in it. Not a lie about being ready, and not a lie
    /// about recharging either: an empty bar with no number is a state the
    /// other two cannot produce.
    const EMPTY: Experience = Experience {
        progress: 0.0,
        level: 0,
    };
    const READY: Experience = Experience {
        progress: 1.0,
        level: 0,
    };

    let Some(recharge) = recharge else {
        return EMPTY;
    };
    // One division rather than three comparisons, so a zero-length cooldown
    // (infinity), a finished one (zero) and a nonsensical one (NaN) all land
    // on "ready" without a branch each.
    let left = recharge.remaining / recharge.full;
    if !left.is_finite() || left <= 0.0 {
        return READY;
    }
    Experience {
        progress: quantise(1.0 - left),
        level: whole_seconds(recharge.remaining),
    }
}

/// `fraction` snapped down to a whole [`METER_STEPS`] step.
///
/// Down and not to nearest, which is the property that matters for the
/// experience bar: one that is still recharging never rounds up to full, so
/// "the bar is full" and "the ability is ready" are the same statement rather
/// than nearly the same one.
///
/// The boss bar goes through this too, for the other reason. Its progress is a
/// continuous quantity in three of the five phases -- a countdown, a prepare
/// timer, and health under a kit's regeneration -- so an exact fraction is a
/// packet every tick per player forever. Measured on the wire by
/// `nix run .#smash-hud-e2e`, which is eight clients through one countdown and
/// into a match: 3,065 boss bar packets in thirty seconds before, 1,135 after,
/// for a bar 182 pixels wide that cannot draw the difference.
fn quantise(fraction: f32) -> f32 {
    let steps = f32::from(METER_STEPS);
    (fraction.clamp(0.0, 1.0) * steps).floor() / steps
}

/// Seconds rounded up, and never below one while there are any left.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a cooldown is seconds, and the longest one in the roster is 30"
)]
fn whole_seconds(seconds: f32) -> i32 {
    seconds.ceil().clamp(1.0, f32::from(i16::MAX)) as i32
}

/// The percentage at which the bar stops being green, and at which it turns
/// red.
///
/// Read off the model rather than chosen for looks. `percent` is
/// `(knockback_multiplier - 1) * 100`, so fifty is a hit that sends you half
/// again as far as it would have at full health and a hundred is one that
/// sends you twice as far. Twice is where a hit that was survivable stops
/// being, which is the moment a player wants the bar to be shouting.
pub const YELLOW_PERCENT: f32 = 50.0;
pub const RED_PERCENT: f32 = 100.0;

/// Which band a percentage falls in.
#[must_use]
pub fn percent_colour(percent: f32) -> BarColour {
    if percent >= RED_PERCENT {
        BarColour::Red
    } else if percent >= YELLOW_PERCENT {
        BarColour::Yellow
    } else {
        BarColour::Green
    }
}

/// The colour a band's number is written in, so the text and the bar say the
/// same thing.
const fn ink(colour: BarColour) -> NamedColor {
    match colour {
        BarColour::Green => NamedColor::Green,
        BarColour::Yellow => NamedColor::Yellow,
        BarColour::Red => NamedColor::Red,
        BarColour::Blue => NamedColor::Aqua,
    }
}

/// Everything one player's bar is decided from.
///
/// A struct and not eight arguments because every field is a number and half
/// of them are counts, which is the shape where an argument list silently
/// swaps two of them.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct View<'a> {
    pub phase: Phase,
    /// Everyone connected.
    pub players: u32,
    /// Everyone still in the match.
    pub alive: u32,
    /// The fewest players a match can start with.
    pub min_players: u32,
    /// The lobby's own clock.
    pub timer: f32,
    /// What that clock started from, for the phases that count down. Zero for
    /// the phases that do not.
    pub span: f32,
    /// This player is out of lives and watching.
    pub eliminated: bool,
    /// Their knockback percentage. See
    /// [`crate::module::knockback::percent`].
    pub percent: f32,
    /// Their health, `0.0..=1.0`.
    pub health: f32,
    /// Who won, once anybody has.
    pub winner: Option<&'a str>,
}

/// The bar across the top of one player's screen.
///
/// Total, so there is no state of the game in which the strip is blank. One
/// bar rather than two, and what it carries is whatever the player should be
/// watching at that moment: before the match, how many more people are needed
/// and then how long is left; during it, the number the mode is about. A
/// second permanent bar for the match clock was the alternative and was
/// dropped, because a twenty minute timeout nobody has ever reached is not
/// something a player acts on and it would have spent the other half of the
/// only always-visible strip on the screen saying so.
#[must_use]
pub fn boss_bar(view: &View<'_>) -> BossBar {
    match view.phase {
        Phase::Waiting => BossBar {
            title: Text::text(format!(
                "Waiting for players  {}/{}",
                view.players, view.min_players
            ))
            .color(NamedColor::Aqua),
            progress: ratio(view.players, view.min_players),
            colour: BarColour::Blue,
        },
        Phase::Countdown => BossBar {
            title: Text::text(format!("Starting in {}s", whole_seconds(view.timer)))
                .color(NamedColor::Yellow),
            progress: fraction(view.timer, view.span),
            colour: BarColour::Yellow,
        },
        Phase::Preparing => BossBar {
            title: Text::text("Get ready").color(NamedColor::Yellow).bold(),
            progress: fraction(view.timer, view.span),
            colour: BarColour::Yellow,
        },
        // A spectator's own percentage is a number about a body that is no
        // longer in the match, so the bar answers the question they do have
        // instead: how much of it is left to watch.
        Phase::Playing if view.eliminated => BossBar {
            title: Text::text(format!("{} still in", view.alive)).color(NamedColor::Aqua),
            progress: ratio(view.alive, view.players),
            colour: BarColour::Blue,
        },
        Phase::Playing => {
            let colour = percent_colour(view.percent);
            BossBar {
                // The number is the point, so it is the whole title and it is
                // bold. The bar under it is the same fact drawn as a length,
                // which is what makes it readable without being read.
                title: Text::text(format!("{}%", round(view.percent)))
                    .color(ink(colour))
                    .bold(),
                // Health and not the percentage, because a bar has to be
                // bounded and a percentage is not: it rises with the kit's
                // health pool and there is no full mark to draw it against.
                // Health has one, and it drains as the percentage climbs,
                // which is the direction a player already reads as bad.
                //
                // Quantised, because every kit regenerates health continuously
                // and an exact fraction is a packet a tick for as long as
                // anybody is below full.
                progress: quantise(view.health),
                colour,
            }
        }
        Phase::Ended => view.winner.map_or_else(
            || BossBar {
                title: Text::text("Nobody was left standing").color(NamedColor::Aqua),
                progress: 1.0,
                colour: BarColour::Blue,
            },
            |winner| BossBar {
                title: Text::text(format!("{winner} wins!"))
                    .color(NamedColor::Gold)
                    .bold(),
                progress: 1.0,
                colour: BarColour::Green,
            },
        ),
    }
}

/// A count as a float. A lobby has fewer players than `u16::MAX`, which is
/// what makes this exact rather than a lossy cast.
fn count(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

/// `part` out of `whole`, clamped, and zero rather than a division by zero.
fn ratio(part: u32, whole: u32) -> f32 {
    fraction(count(part), count(whole))
}

/// The same for two times, snapped to a step so a timer that moves every tick
/// does not send a packet every tick. See [`quantise`].
fn fraction(part: f32, whole: f32) -> f32 {
    let value = part / whole;
    if value.is_finite() {
        quantise(value)
    } else {
        0.0
    }
}

/// A percentage as a whole number.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "percent is non-negative by construction and bounded by the kit's health pool"
)]
const fn round(percent: f32) -> u32 {
    percent.max(0.0).round() as u32
}

/// The last seconds of the countdown that get a number across the screen.
///
/// Three, where five get a sound. A digit in the middle of the screen is the
/// most intrusive thing this file draws, so it earns its place only for the
/// seconds a player is actually timing something against. The sound has no such
/// cost and starts earlier.
pub const COUNTDOWN_TITLE_SECONDS: f32 = 3.0;

/// The number in the middle of the screen with `seconds` to go, if this is one
/// of the seconds that gets one.
///
/// `Preparing` only, and not the lobby countdown, even though both phases count
/// down and both tick out loud. Two phases run out in a row -- the lobby's
/// clock into `Preparing`, and `Preparing`'s nine seconds into the match -- and
/// a window on both is two "3, 2, 1" sequences back to back with a teleport
/// between them. The wire gate caught exactly that: `['3', '2', '1', '3', '2',
/// '1', 'GO!']`. Only the second sequence counts down to anything a player can
/// act on, so only the second one gets digits; the lobby's wait is up to sixty
/// seconds and belongs on the bar, which can say "Starting in 47s" without
/// filling the screen.
#[must_use]
pub fn countdown_title(phase: Phase, seconds: f32) -> Option<Title> {
    if phase != Phase::Preparing || seconds <= 0.0 || seconds > COUNTDOWN_TITLE_SECONDS {
        return None;
    }
    Some(
        Title::new(
            Text::text(whole_seconds(seconds).to_string())
                .color(NamedColor::Gold)
                .bold(),
        )
        .under(Text::text("Get ready").color(NamedColor::Gray))
        // Exactly one second, so each digit is replaced by the next rather
        // than fading across it.
        .timed(TitleTimes::TICK),
    )
}

/// The title a phase change puts on screen, if it puts one there.
///
/// Two of the five phases do. `Countdown` and `Preparing` are covered by
/// [`countdown_title`] and by the bar, and `Waiting` is the hub, where a title
/// across the screen would be permanent furniture.
#[must_use]
pub fn phase_title(to: Phase, winner: Option<&str>) -> Option<Title> {
    match to {
        Phase::Playing => Some(
            Title::new(Text::text("GO!").color(NamedColor::Green).bold())
                .under(Text::text("Smash them off the map").color(NamedColor::Gray))
                .timed(TitleTimes::TICK),
        ),
        Phase::Ended => Some(winner.map_or_else(
            || {
                Title::new(Text::text("Game over").color(NamedColor::Red).bold())
                    .under(Text::text("Nobody was left standing").color(NamedColor::Gray))
            },
            |winner| {
                Title::new(
                    Text::text(format!("{winner} wins!"))
                        .color(NamedColor::Gold)
                        .bold(),
                )
                .under(Text::text("Last mob standing").color(NamedColor::Gray))
            },
        )),
        Phase::Waiting | Phase::Countdown | Phase::Preparing => None,
    }
}

/// What a player sees on their own screen when they die.
///
/// The subtitle is the point. The chat line naming both of you is broadcast
/// and scrolls away behind the next one, and the middle of your own screen is
/// where you are looking at the instant you are launched off the map: before
/// this it carried a life count and no answer at all to the only question a
/// player asks there, which is who did that. The credit is not new -- the
/// damage pipeline has recorded `(LastHitBy, attacker)` all along -- it simply
/// had nowhere to be read.
#[must_use]
pub fn death_title(lives_left: u8, killer: Option<&str>, cause: DeathCause) -> Title {
    let heading = match lives_left {
        0 => Text::text("GAME OVER").color(NamedColor::Red).bold(),
        // Not "1 lives left". The line is read at the worst moment of a
        // match and a grammatical error is exactly the sort of thing that
        // makes a game feel unfinished.
        1 => Text::text("1 life left!").color(NamedColor::Red),
        left => Text::text(format!("{left} lives left!")).color(NamedColor::Gold),
    };
    let under = match (killer, cause) {
        (Some(killer), _) => Text::text(format!("Smashed by {killer}")),
        (None, DeathCause::Void) => Text::text("You fell out of the world"),
        (None, DeathCause::Damage) => Text::text("Nobody to blame but yourself"),
    };
    Title::new(heading).under(under.color(NamedColor::Gray))
}

/// Who won, from everyone's remaining lives.
///
/// The last player standing in the ordinary case, where exactly one person has
/// any lives left. A match that runs into the twenty minute timeout ends with
/// several people alive and the honest answer there is whoever has the most,
/// which is the same rule; when two are level there is no winner and the
/// results screen says so rather than picking one of them.
#[must_use]
pub fn winner_of(standings: &[(String, u8)]) -> Option<&str> {
    let best = standings
        .iter()
        .map(|(_, lives)| *lives)
        .filter(|lives| *lives > 0)
        .max()?;
    let mut leaders = standings.iter().filter(|(_, lives)| *lives == best);
    let (name, _) = leaders.next()?;
    if leaders.next().is_some() {
        return None;
    }
    Some(name)
}

/// Everyone's name and remaining lives, as [`winner_of`] wants them.
#[must_use]
pub fn standings(world: WorldRef<'_>) -> Vec<(String, u8)> {
    let mut rows = Vec::new();
    world
        .query::<&Lives>()
        .with(Player::id())
        .build()
        .each_entity(|player, lives| rows.push((player.name(), lives::remaining(player, *lives))));
    rows
}

/// Who has won the match that is ending, if anybody has.
#[must_use]
pub fn winner(world: WorldRef<'_>) -> Option<String> {
    let rows = standings(world);
    winner_of(&rows).map(ToOwned::to_owned)
}

/// What this client was last told.
///
/// Initialised to the state a client is already in when it joins: a fresh
/// client's experience bar is empty with no level, and it has no boss bar
/// until something adds one. So the first tick compares against the truth
/// rather than against a guess, and a player who joins gets exactly one push
/// of each rather than none or two.
///
/// Since the boss bar became `hyperion::egress::boss_bar`, which diffs per
/// viewer against what actually went on the wire, this half of `Shown` no
/// longer decides whether a packet is sent. It is not therefore redundant, and
/// the two are not the same rule written twice: this one decides whether a
/// *text component is cloned and queued*, twenty times a second per player, on
/// a value that is usually the same as last tick. The wire diff cannot help
/// with that, because by the time it runs the allocation has happened. The
/// experience bar has no wire diff at all and needs this one for both jobs,
/// which is the other reason the two fields stay in one struct.
#[derive(Component, Debug, Clone, PartialEq, Default)]
struct Shown {
    experience: Experience,
    /// `None` before the first push, which is also the client's own state.
    bar: Option<BossBar>,
}

/// One player's push, held until the query that computed it has finished.
///
/// The bar is boxed because it carries a text component and the meter carries
/// two numbers, so an unboxed enum would size every element of the queue to the
/// larger of the two.
enum Push {
    Experience(PlayerId, Experience),
    Bar(PlayerId, Box<BossBar>),
}

/// What the phase's timer started from, so the bar can draw it as a fraction.
fn span_of(config: &LobbyConfig, phase: Phase, players: u32) -> f32 {
    match phase {
        Phase::Countdown => config
            .countdown_for(players)
            .unwrap_or(config.countdown_at_min),
        Phase::Preparing => config.prepare_seconds,
        Phase::Ended => config.results_seconds,
        Phase::Waiting | Phase::Playing => 0.0,
    }
}

/// The cooldown state of one ability instance.
fn recharge_of(ability: EntityView<'_>) -> Recharge {
    Recharge {
        remaining: ability
            .try_get::<&Cooldown>(|cooldown| cooldown.remaining)
            .unwrap_or(0.0),
        full: ability
            .try_get::<&CooldownSpec>(|spec| spec.0)
            .unwrap_or(0.0),
    }
}

#[derive(Component)]
pub struct HudModule;

impl Module for HudModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Hud");

        world.component::<Shown>();
        // This module is what makes the two surfaces mean anything, so this
        // module is what says every player has a record of what is on them.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Shown)>();

        world.system_named::<()>("update_hud").run(|mut it| {
            while it.next() {
                let world = it.world();
                let lobby = world.cloned::<&Lobby>();
                let model = world.cloned::<&KnockbackModel>();
                let players = lobby::player_count(&world);
                let alive = lobby::alive_count(&world);
                let (min_players, span) = world.get::<&LobbyConfig>(|config| {
                    (config.min_players, span_of(config, lobby.phase, players))
                });
                // Only at the end, and once for the whole world rather than
                // once per viewer: it is a query over every player, and the
                // answer is the same on everybody's screen.
                let champion = (lobby.phase == Phase::Ended)
                    .then(|| winner(world))
                    .flatten();

                let mut pushes = Vec::new();
                world
                    .query::<(&PlayerId, &Health, &SelectedSlot, &mut Shown)>()
                    .with(Player::id())
                    .build()
                    .each_entity(|player, (id, health, slot, shown)| {
                        let experience =
                            meter(ability::granted_in_slot(player, slot.0).map(recharge_of));
                        let bar = boss_bar(&View {
                            phase: lobby.phase,
                            players,
                            alive,
                            min_players,
                            timer: lobby.timer,
                            span,
                            eliminated: player.has(Eliminated::id()),
                            percent: knockback::percent(model, *health),
                            health: health.fraction(),
                            winner: champion.as_deref(),
                        });

                        if shown.experience != experience {
                            shown.experience = experience;
                            pushes.push(Push::Experience(*id, experience));
                        }
                        if shown.bar.as_ref() != Some(&bar) {
                            shown.bar = Some(bar.clone());
                            pushes.push(Push::Bar(*id, Box::new(bar)));
                        }
                    });

                if pushes.is_empty() {
                    continue;
                }
                world.get::<&ServerHandle>(|server| {
                    for push in pushes {
                        match push {
                            Push::Experience(id, experience) => {
                                server.set_experience(id, experience);
                            }
                            Push::Bar(id, bar) => server.set_boss_bar(id, *bar),
                        }
                    }
                });
            }
        });
    }
}
