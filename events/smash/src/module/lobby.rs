//! Hub, queue, countdown, match, results, back to hub.
//!
//! The whole flow is one small state machine and [`step`] is all of it: a pure
//! function from the current phase, a time delta and two counts to the next
//! phase. Everything with a side effect — announcing, scattering players onto
//! spawns, flipping people into spectator — hangs off the transition rather
//! than being tangled into it, so `tests/lobby.rs` can drive every edge without
//! a world.
//!
//! Countdown lengths are Mineplex's generic arcade ones: full lobby ten
//! seconds, three-quarters full thirty, minimum sixty, and dropping back below
//! the minimum cancels.

use std::borrow::Cow;

use flecs_ecs::prelude::*;

use crate::{
    module::{
        arena::Arena,
        damage::MatchClock,
        hud,
        kit::{self, KitName},
        lives::{Eliminated, InvulnerableUntil, Lives, Placement, RespawnAt},
        player::{Health, Player, Position},
        selector, sound,
        vitals::{Hunger, VitalsComponentsModule},
    },
    server::{Channel, NamedColor, PlayerId, ServerHandle, Sound, SoundCategory, Text},
};

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Not enough players. The hub.
    #[default]
    Waiting,
    /// Enough players; the clock is running and can still be cancelled.
    Countdown,
    /// Committed. Players are on the map, abilities are locked.
    Preparing,
    Playing,
    /// Scoreboard is up, nobody can act.
    Ended,
}

/// Thresholds and durations. A singleton so a server can run a two-player
/// duels variant without touching the state machine.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct LobbyConfig {
    pub min_players: u32,
    pub full_players: u32,
    pub countdown_at_min: f32,
    pub countdown_at_three_quarters: f32,
    pub countdown_at_full: f32,
    /// Mineplex's `PrepareTime`.
    pub prepare_seconds: f32,
    /// Mineplex's `GameTimeout`. There is no sudden death; the hunger drain is
    /// what stops a stalled match instead.
    pub match_timeout_seconds: f32,
    pub results_seconds: f32,
}

impl Default for LobbyConfig {
    /// Mineplex ran Super Smash Mobs at 4 to start and 8 for a full lobby.
    /// We start at 2 and call 4 full, because the people playing this server
    /// are the two or three developing it, and a threshold nobody can reach
    /// is a game nobody can test.
    ///
    /// The three countdown lengths stay distinct at these numbers: 2 players
    /// takes `countdown_at_min`, 3 satisfies the three-quarters rule, and 4
    /// is full. Lowering `full_players` below 4 would collapse two of those
    /// bands into one.
    fn default() -> Self {
        Self {
            min_players: 2,
            full_players: 4,
            countdown_at_min: 60.0,
            countdown_at_three_quarters: 30.0,
            countdown_at_full: 10.0,
            prepare_seconds: 9.0,
            match_timeout_seconds: 20.0 * 60.0,
            results_seconds: 10.0,
        }
    }
}

/// Read a player count from the environment, or keep `fallback`.
///
/// A value that is set but unreadable stops the server. It is a typo in a
/// number the whole match schedule turns on, and a lobby quietly running
/// thresholds nobody asked for is worse than one that does not boot.
fn threshold_from_env(name: &str, fallback: u32) -> u32 {
    std::env::var(name).map_or(fallback, |raw| {
        raw.parse().unwrap_or_else(|error| {
            panic!("{name} is not a player count: {raw:?} ({error})");
        })
    })
}

impl LobbyConfig {
    /// The lobby this server is configured to run.
    ///
    /// `SMASH_MIN_PLAYERS` and `SMASH_FULL_PLAYERS` set the two thresholds, in
    /// the same `SMASH_`-prefixed namespace the binary already reads its
    /// address and its certificates from.
    ///
    /// This is the seam the type's own comment describes. The counts are the
    /// part of a lobby deployments actually differ on -- a duels box, a full
    /// house, a soak run -- and each wants its own without a rebuild and
    /// without touching the state machine. The durations are not exposed,
    /// because nobody has yet wanted them to differ.
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let config = Self {
            min_players: threshold_from_env("SMASH_MIN_PLAYERS", defaults.min_players),
            full_players: threshold_from_env("SMASH_FULL_PLAYERS", defaults.full_players),
            ..defaults
        };
        assert!(
            config.min_players >= 1,
            "SMASH_MIN_PLAYERS is 0, so an empty lobby would start a match"
        );
        assert!(
            config.full_players >= config.min_players,
            "SMASH_FULL_PLAYERS is {} and SMASH_MIN_PLAYERS is {}, so the lobby would be full \
             before it was allowed to start",
            config.full_players,
            config.min_players,
        );
        config
    }

    /// How long the countdown should be for this many players, or `None` if
    /// there are not enough to run one.
    ///
    /// The minimum is checked first and alone. It used to be the last of three
    /// branches, which let the three-quarters rule answer for a lobby that had
    /// not reached its minimum at all: at `min_players: 4, full_players: 4`
    /// three players satisfy `3 * 4 >= 4 * 3` and started a countdown under a
    /// minimum of four. The field is named `min_players`, so it is a minimum;
    /// the bands below only choose how long.
    #[must_use]
    pub const fn countdown_for(&self, players: u32) -> Option<f32> {
        if players < self.min_players {
            return None;
        }
        if players >= self.full_players {
            Some(self.countdown_at_full)
        } else if players * 4 >= self.full_players * 3 {
            Some(self.countdown_at_three_quarters)
        } else {
            Some(self.countdown_at_min)
        }
    }
}

/// The live state. A singleton.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Lobby {
    pub phase: Phase,
    /// Seconds remaining in [`Phase::Countdown`], [`Phase::Preparing`] and
    /// [`Phase::Ended`]; seconds elapsed in [`Phase::Playing`].
    pub timer: f32,
}

/// One tick of the state machine. Pure.
///
/// `players` is everyone connected; `alive` is everyone not yet eliminated.
#[must_use]
pub fn step(config: &LobbyConfig, state: Lobby, dt: f32, players: u32, alive: u32) -> Lobby {
    match state.phase {
        Phase::Waiting => config
            .countdown_for(players)
            .map_or(state, |seconds| Lobby {
                phase: Phase::Countdown,
                timer: seconds,
            }),
        Phase::Countdown => {
            let Some(target) = config.countdown_for(players) else {
                // Someone left. Back to the hub, and the clock is discarded
                // rather than paused: Mineplex's `SetCountdown(-1)`.
                return Lobby {
                    phase: Phase::Waiting,
                    timer: 0.0,
                };
            };
            // A join that fills the lobby shortens the wait immediately; it
            // never lengthens it.
            let timer = (state.timer - dt).min(target);
            if timer <= 0.0 {
                Lobby {
                    phase: Phase::Preparing,
                    timer: config.prepare_seconds,
                }
            } else {
                Lobby {
                    phase: Phase::Countdown,
                    timer,
                }
            }
        }
        Phase::Preparing => {
            let timer = state.timer - dt;
            if timer <= 0.0 {
                Lobby {
                    phase: Phase::Playing,
                    timer: 0.0,
                }
            } else {
                Lobby {
                    phase: Phase::Preparing,
                    timer,
                }
            }
        }
        Phase::Playing => {
            let timer = state.timer + dt;
            if alive <= 1 || timer >= config.match_timeout_seconds {
                Lobby {
                    phase: Phase::Ended,
                    timer: config.results_seconds,
                }
            } else {
                Lobby {
                    phase: Phase::Playing,
                    timer,
                }
            }
        }
        Phase::Ended => {
            let timer = state.timer - dt;
            if timer <= 0.0 {
                Lobby {
                    phase: Phase::Waiting,
                    timer: 0.0,
                }
            } else {
                Lobby {
                    phase: Phase::Ended,
                    timer,
                }
            }
        }
    }
}

/// Emitted at the world when the phase changes.
#[derive(Component, Debug, Copy, Clone)]
pub struct PhaseChanged {
    pub from: Phase,
    pub to: Phase,
}

/// Pick a kit in the hub, by name.
///
/// # Errors
/// As [`choose`], plus a name that is not a kit.
pub fn select_kit(world: &World, player: EntityView<'_>, name: &str) -> Result<(), String> {
    let Some(chosen) = kit::by_name(world, name) else {
        return Err("No such kit.".to_owned());
    };
    choose(world, player, chosen)
}

/// Pick a kit in the hub.
///
/// Two rules, and both of them live here rather than on either of the two
/// surfaces a player reaches them through, so a podium click and `/kit` cannot
/// come to different answers:
///
/// * Not once the match has committed. This is the only rule the lobby imposes
///   on kits, and it is also what makes a claim last the rest of the match
///   without anything having to say so: past [`Phase::Countdown`] nothing can
///   change a kit at all.
/// * Not a mob somebody else is already playing. Derived from their
///   `(Playing, kit)` edge on every call, so a player who disconnects frees
///   their mob with no cleanup anywhere. See [`crate::module::selector`] for
///   why this rule exists and why it is this server's own and not a
///   reconstruction of anything.
///
/// # Errors
/// If the phase is past [`Phase::Countdown`], or somebody else holds the kit.
pub fn choose(world: &World, player: EntityView<'_>, chosen: EntityView<'_>) -> Result<(), String> {
    let phase = world.cloned::<&Lobby>().phase;
    if !matches!(phase, Phase::Waiting | Phase::Countdown) {
        return Err("You cannot change kit once the game has started.".to_owned());
    }
    if let Some(holder) = kit::claimant(world, chosen.id())
        && holder != player.id()
    {
        return Err(selector::taken_message(world, chosen, holder));
    }

    let name = chosen.try_get::<&KitName>(|name| name.0).unwrap_or("");
    kit::apply(world, player, chosen);

    // The kit that was chosen, not the kit the player is on: this runs inside a
    // deferred world, where `apply`'s `(Playing, kit)` edge has been queued and
    // not applied, so asking the player what they are playing answers with what
    // they were playing before. See `sound::play_declared_to`. Nothing here
    // names a kit or looks a sound up; the mob answers for itself, to the player
    // who chose it and to nobody else.
    sound::play_declared_to(world.into(), chosen, sound::PlaysOnSelect, player);

    if let Some(id) = player.try_get::<&PlayerId>(|p| *p) {
        let hotbar = kit::hotbar(player);
        world.get::<&ServerHandle>(|server| {
            server.set_hotbar(id, &hotbar);
            server.send_message(id, Channel::Chat, Text::text(format!("Kit set to {name}.")));
            // The same channel the refusal uses, so a player watching one spot
            // on the screen sees both answers there.
            server.send_message(
                id,
                Channel::ActionBar,
                Text::text(format!("You are the {name}.")).color(NamedColor::Green),
            );
        });
    }
    Ok(())
}

/// Everyone who has not been eliminated.
#[must_use]
pub fn alive_count(world: &World) -> u32 {
    let count = world
        .query::<()>()
        .with(Player::id())
        .without(Eliminated::id())
        .build()
        .count();
    u32::try_from(count).unwrap_or(0)
}

#[must_use]
pub fn player_count(world: &World) -> u32 {
    let count = world.query::<()>().with(Player::id()).build().count();
    u32::try_from(count).unwrap_or(0)
}

#[derive(Component)]
pub struct LobbyModule;

impl Module for LobbyModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Lobby");

        // The end of a match refills every food bar, so the component that
        // holds one has to be registered before `reset` first queries for it.
        world.import::<VitalsComponentsModule>();

        world.component::<Lobby>().add_trait::<flecs::Singleton>();
        world
            .component::<LobbyConfig>()
            .add_trait::<flecs::Singleton>();
        world.component::<PhaseChanged>();
        world.set(Lobby::default());
        world.set(LobbyConfig::from_env());

        world.system_named::<()>("lobby_tick").run(|mut it| {
            while it.next() {
                let world = it.world();
                let dt = it.delta_time();
                let players = player_count(&world);
                let alive = alive_count(&world);

                let (before, after) = world.get::<(&mut Lobby, &LobbyConfig)>(|(lobby, config)| {
                    let before = *lobby;
                    *lobby = step(config, before, dt, players, alive);
                    (before, *lobby)
                });

                if after.phase == Phase::Playing {
                    world.get::<&mut MatchClock>(|clock| clock.0 += dt);
                }

                tick_countdown(&world, before, after);

                if before.phase != after.phase {
                    on_phase_change(&world, before.phase, after.phase);
                }
            }
        });
    }
}

/// Whether a phase is one whose timer is a countdown to something.
///
/// Both of the phases before a match are: the lobby countdown runs out into
/// `Preparing`, and `Preparing` runs out into the match itself. `Playing`
/// counts *up* and `Ended` is a results screen nobody is waiting on, so neither
/// is ticked.
const fn counts_down(phase: Phase) -> bool {
    matches!(phase, Phase::Countdown | Phase::Preparing)
}

/// A tick a second through the last seconds before the match, to everyone.
///
/// Driven off the whole-second boundary the timer just crossed rather than off
/// a counter of its own, so it stays right when a join shortens the countdown
/// and cannot drift from the number the rest of the game is working to. A
/// transition between the two counting-down phases is not a boundary crossing:
/// `Preparing` starts at nine seconds having just been handed a fresh timer,
/// and treating that as a tick would double up on the handover.
fn tick_countdown(world: &WorldRef<'_>, before: Lobby, after: Lobby) {
    if !counts_down(after.phase) || before.phase != after.phase {
        return;
    }
    let Some(seconds_left) = sound::countdown_second_crossed(before.timer, after.timer) else {
        return;
    };
    sound::play_to_everyone(*world, sound::countdown_tick(seconds_left));
    // The same boundary drives the number on screen, so the digit and the tick
    // that goes with it cannot come from two clocks and disagree. Which of
    // those seconds gets a digit is `hud`'s to say, and it is fewer of them than
    // get a sound: both phases here tick out loud and only one of them shows a
    // number.
    if let Some(title) = hud::countdown_title(after.phase, seconds_left) {
        world.get::<&ServerHandle>(|server| server.broadcast_title(title));
    }
}

fn on_phase_change(world: &WorldRef<'_>, from: Phase, to: Phase) {
    // Read before the transition's own side effects, because `reset` is one of
    // them and it is what puts everybody's lives back.
    let champion = if to == Phase::Ended {
        hud::winner(*world)
    } else {
        None
    };

    match to {
        Phase::Countdown => announce(world, "Enough players. The game starts shortly."),
        Phase::Preparing => {
            scatter(world);
            announce(world, "Get ready!");
        }
        Phase::Playing => {
            world.get::<&mut MatchClock>(|clock| clock.0 = 0.0);
            announce(world, "Go!");
            sound::play_to_everyone(
                *world,
                Sound::new(sound::MATCH_START, SoundCategory::Master),
            );
        }
        Phase::Ended => {
            // Chat is the record, so it carries the result and not just the
            // fact that there was one. A player who was reading the panel when
            // the last hit landed can scroll back to this line; the title that
            // goes with it is gone in five seconds.
            match champion.as_deref() {
                Some(winner) => announce(world, format!("Game over. {winner} wins!")),
                None => announce(world, "Game over. Nobody was left standing."),
            }
            sound::play_to_everyone(*world, Sound::new(sound::MATCH_END, SoundCategory::Ui));
        }
        Phase::Waiting => reset(world),
    }

    // After the announcement and the sound, so the three arrive in the order a
    // player reads them: the chat line is the record, the noise is the cue,
    // and the title is the thing they are looking at.
    if let Some(title) = hud::phase_title(to, champion.as_deref()) {
        world.get::<&ServerHandle>(|server| server.broadcast_title(title));
    }

    world
        .event()
        .add(Lobby::id())
        .entity(world.component::<Lobby>())
        .emit(&PhaseChanged { from, to });
}

/// One line to everybody's chat.
///
/// `Cow` rather than `&'static str` so the results line can name the winner.
/// Deliberately unstyled: `Channel::Chat` cannot carry a style at all, and the
/// adapter logs a warning for anything that tries, so a colour here would be a
/// silent no-op with a log line nobody reads. ENG-10796 is the fix.
fn announce(world: &WorldRef<'_>, text: impl Into<Cow<'static, str>>) {
    world.get::<&ServerHandle>(|server| server.broadcast(Channel::Chat, Text::text(text)));
}

/// Put everyone on a spawn point and give them their kit's hotbar.
fn scatter(world: &WorldRef<'_>) {
    let arena = world.cloned::<&Arena>();
    let mut index = 0u64;

    let mut placed = Vec::new();
    world
        .query::<&PlayerId>()
        .with(Player::id())
        .build()
        .each_entity(|player, id| {
            let at = arena.spawn(index);
            index += 1;
            player.set(Position(at));
            placed.push((player.id(), *id, at));
        });

    world.get::<&ServerHandle>(|server| {
        for (entity, id, at) in &placed {
            server.teleport(*id, *at);
            server.set_spectating(*id, false);
            let hotbar = kit::hotbar(world.entity_from_id(*entity));
            server.set_hotbar(*id, &hotbar);
        }
    });
}

/// Clear the match state so the same world can host the next game.
///
/// Everything a match writes onto a player has to come off here, not just the
/// two things a player can see. The three removals below are each a real bug if
/// they are missing, and none of them shows up until the second match:
///
/// * A stale [`Placement`] is a finishing position from the last game sitting
///   on somebody who has not finished this one, which is what a results screen
///   reads.
/// * A stale [`RespawnAt`] is measured against a clock this function has just
///   set back to zero, so a player who died in the closing seconds of one match
///   spends the opening of the next one dead and spectating, for however long
///   the old clock said.
/// * A stale [`InvulnerableUntil`] is the same arithmetic the other way: it
///   makes them untouchable, and immune to the kill plane, for that long
///   instead.
///
/// The food bar is the fourth. Hunger is deliberately *not* refilled by dying
/// -- an anti-stall clock a player can reset by throwing a life away is not a
/// clock -- so the only thing that ever refills it is landing a hit, and the
/// end of a match. Without this a second match starts with whatever everyone
/// had left, and somebody spawns already starving.
fn reset(world: &WorldRef<'_>) {
    world.get::<&mut MatchClock>(|clock| clock.0 = 0.0);
    // Refilling the bar is half the job; the client draws whatever it was last
    // sent, so a refill nobody is told about leaves everyone looking at the
    // drained bar from the previous match until the next drain tick moves it.
    // Collected here and pushed below rather than from inside the query, which
    // is holding `Health` open for the damage path the seam can reach.
    let mut refilled = Vec::new();
    world
        .query::<(&mut Lives, &mut Health, &mut Hunger, &PlayerId)>()
        .with(Player::id())
        .build()
        .each_entity(|player, (lives, health, hunger, id)| {
            *lives = Lives::default();
            health.current = health.max;
            *hunger = Hunger::full(hunger.interval);
            refilled.push((*id, hunger.food));
            player.remove(Eliminated::id());
            player.remove(Placement::id());
            player.remove(RespawnAt::id());
            player.remove(InvulnerableUntil::id());
        });
    world.get::<&ServerHandle>(|server| {
        for (id, food) in refilled {
            server.set_food(id, food);
        }
    });
}
