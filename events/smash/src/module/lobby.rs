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

use flecs_ecs::prelude::*;

use crate::{
    module::{
        arena::Arena,
        damage::MatchClock,
        kit,
        lives::{Eliminated, Lives},
        player::{Health, Player, Position},
    },
    server::{Channel, PlayerId, ServerHandle},
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
    fn default() -> Self {
        Self {
            min_players: 4,
            full_players: 8,
            countdown_at_min: 60.0,
            countdown_at_three_quarters: 30.0,
            countdown_at_full: 10.0,
            prepare_seconds: 9.0,
            match_timeout_seconds: 20.0 * 60.0,
            results_seconds: 10.0,
        }
    }
}

impl LobbyConfig {
    /// How long the countdown should be for this many players, or `None` if
    /// there are not enough to run one.
    #[must_use]
    pub fn countdown_for(&self, players: u32) -> Option<f32> {
        if players >= self.full_players {
            Some(self.countdown_at_full)
        } else if players * 4 >= self.full_players * 3 {
            Some(self.countdown_at_three_quarters)
        } else if players >= self.min_players {
            Some(self.countdown_at_min)
        } else {
            None
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
        Phase::Waiting => match config.countdown_for(players) {
            Some(seconds) => Lobby {
                phase: Phase::Countdown,
                timer: seconds,
            },
            None => state,
        },
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

/// Pick a kit in the hub. Refused once the match has committed, which is the
/// one rule the lobby imposes on kits.
///
/// # Errors
/// If the phase is past [`Phase::Countdown`], or the name is not a kit.
pub fn select_kit(world: &World, player: EntityView<'_>, name: &str) -> Result<(), &'static str> {
    let phase = world.cloned::<&Lobby>().phase;
    if !matches!(phase, Phase::Waiting | Phase::Countdown) {
        return Err("You cannot change kit once the game has started.");
    }
    let Some(chosen) = kit::by_name(world, name) else {
        return Err("No such kit.");
    };
    kit::apply(world, player, chosen);

    if let Some(id) = player.try_get::<&PlayerId>(|p| *p) {
        let hotbar = kit::hotbar(player);
        world.get::<&ServerHandle>(|server| {
            server.set_hotbar(id, &hotbar);
            server.send_message(id, Channel::Chat, &format!("Kit set to {name}."));
        });
    }
    Ok(())
}

/// Everyone who has not been eliminated.
#[must_use]
pub fn alive_count(world: &World) -> u32 {
    world
        .query::<()>()
        .with(Player::id())
        .without(Eliminated::id())
        .build()
        .count() as u32
}

#[must_use]
pub fn player_count(world: &World) -> u32 {
    world.query::<()>().with(Player::id()).build().count() as u32
}

#[derive(Component)]
pub struct LobbyModule;

impl Module for LobbyModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Lobby");

        world.component::<Lobby>().add_trait::<flecs::Singleton>();
        world
            .component::<LobbyConfig>()
            .add_trait::<flecs::Singleton>();
        world.component::<PhaseChanged>();
        world.set(Lobby::default());
        world.set(LobbyConfig::default());

        world.system_named::<()>("smash::lobby_tick").run(|mut it| {
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

                if before.phase != after.phase {
                    on_phase_change(&world, before.phase, after.phase);
                }
            }
        });
    }
}

fn on_phase_change(world: &WorldRef<'_>, from: Phase, to: Phase) {
    match to {
        Phase::Countdown => announce(world, "Enough players. The game starts shortly."),
        Phase::Preparing => {
            scatter(world);
            announce(world, "Get ready!");
        }
        Phase::Playing => {
            world.get::<&mut MatchClock>(|clock| clock.0 = 0.0);
            announce(world, "Go!");
        }
        Phase::Ended => announce(world, "Game over."),
        Phase::Waiting => reset(world),
    }

    world
        .event()
        .add(Lobby::id())
        .entity(world.component::<Lobby>())
        .emit(&PhaseChanged { from, to });
}

fn announce(world: &WorldRef<'_>, text: &str) {
    world.get::<&ServerHandle>(|server| server.broadcast(Channel::Chat, text));
}

/// Put everyone on a spawn point and give them their kit's hotbar.
fn scatter(world: &WorldRef<'_>) {
    let arena = world.cloned::<&Arena>();
    let mut index = 0usize;

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
fn reset(world: &WorldRef<'_>) {
    world.get::<&mut MatchClock>(|clock| clock.0 = 0.0);
    world
        .query::<(&mut Lives, &mut Health)>()
        .with(Player::id())
        .build()
        .each_entity(|player, (lives, health)| {
            *lives = Lives::default();
            health.current = health.max;
            player.remove(Eliminated::id());
        });
}
