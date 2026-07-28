//! A whole game in a test, with no Minecraft server anywhere near it.
//!
//! Three things live here, and the last two are what let a generated test say
//! anything a hand written one cannot:
//!
//! * [`Game`], a world with the game and a recording server in it.
//! * [`Script`], a list of [`Action`]s a test can generate rather than write.
//!   The same script drives a property test and a replay, so a determinism
//!   failure and an invariant failure are found by one driver.
//! * [`invariants`], the things that must be true of the world at the end of
//!   every tick. [`Game::run`] checks them after each one, so a generated
//!   script reports the *first* tick that broke a rule rather than a final
//!   state that is wrong for reasons nobody can reconstruct.

// Each test binary compiles its own copy and uses a different part of it.
#![allow(dead_code)]

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use flecs_ecs::prelude::*;
use glam::Vec3;
use smash::{
    SmashModule,
    module::{
        ability::{self, Cooldown, Grants, Slot},
        damage::{DamageKind, Damaged, MatchClock, hurt},
        kit,
        knockback::Knockback,
        lives::{Eliminated, InvulnerableUntil, Lives, Placement, RespawnAt},
        lobby::{Lobby, LobbyConfig},
        player::{Energy, Facing, Health, JumpsLeft, OnGround, Player, Position, Velocity},
    },
    server::{PlayerId, ServerHandle, mock::MockServer},
};

pub struct Game {
    pub world: World,
    pub server: Arc<MockServer>,
    next_id: u64,
}

impl Game {
    pub fn new() -> Self {
        let world = World::new();
        let server = Arc::new(MockServer::new());
        // Import before set: the workspace enables flecs_manual_registration,
        // so ServerHandle must be registered -- which the module does -- before
        // anything uses it.
        world.import::<SmashModule>();
        world.set(ServerHandle(server.clone()));
        Self {
            world,
            server,
            next_id: 1,
        }
    }

    /// A player standing at `at`, facing +X, on the ground.
    pub fn player(&mut self, name: &str, at: Vec3) -> Entity {
        let id = PlayerId(self.next_id);
        self.next_id += 1;
        self.world
            .entity_named(name)
            .set(id)
            .add(Player::id())
            .set(Position(at))
            .set(Facing(Vec3::X))
            .set(OnGround(true))
            .id()
    }

    /// Advance the simulation by `seconds`, in `steps` equal ticks.
    pub fn advance(&self, seconds: f32, steps: u32) {
        let dt = seconds / steps as f32;
        for _ in 0..steps {
            self.world.progress_time(dt);
        }
    }

    /// Every player, ordered by [`PlayerId`] so a caller can index them the
    /// same way in two different worlds.
    pub fn players(&self) -> Vec<Entity> {
        let mut found: Vec<(u64, Entity)> = Vec::new();
        self.world
            .query::<&PlayerId>()
            .with(Player::id())
            .build()
            .each_entity(|entity, id| found.push((id.0, entity.id())));
        found.sort_unstable_by_key(|(id, _)| *id);
        found.into_iter().map(|(_, entity)| entity).collect()
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// The tick length every generated test uses. Twenty ticks a second, which is
/// Minecraft's.
pub const TICK: f32 = 0.05;

/// One thing a script can do to the world.
///
/// Deliberately small and total: every variant is applicable to every world,
/// with player and kit indices taken modulo what exists, so a generated script
/// never has to be filtered for validity and the shrinker never produces a
/// case that fails for being nonsense rather than for finding a bug.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Advance one tick.
    Tick,
    /// Deal damage from one player to another.
    Hit {
        attacker: usize,
        victim: usize,
        amount: f32,
        kind: u8,
    },
    /// Right-click a hotbar slot.
    UseSlot { player: usize, slot: u8 },
    /// Teleport, as the host's mirror would.
    Move { player: usize, to: Vec3 },
    /// Land or leave the ground.
    Ground { player: usize, on: bool },
    /// Pick a kit, by index into the registry.
    SelectKit { player: usize, kit: usize },
}

impl Action {
    const fn kind(raw: u8) -> DamageKind {
        match raw % 4 {
            0 => DamageKind::Melee,
            1 => DamageKind::Projectile,
            2 => DamageKind::Ability,
            _ => DamageKind::Environment,
        }
    }
}

/// A run of the game, as data.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// How many players to seed the world with.
    pub players: usize,
    pub actions: Vec<Action>,
}

impl Game {
    /// Build the world a script asks for.
    ///
    /// The lobby is left on its real state machine rather than forced into
    /// [`Phase::Playing`], because the transitions are part of what is being
    /// tested. The durations are shortened so a script of a few hundred ticks
    /// reaches the end of a match instead of spending all of them in a
    /// countdown.
    pub fn from_script(script: &Script) -> Self {
        let mut game = Self::new();
        game.world.set(LobbyConfig {
            min_players: 2,
            full_players: 4,
            countdown_at_min: 1.0,
            countdown_at_three_quarters: 0.75,
            countdown_at_full: 0.5,
            prepare_seconds: 0.5,
            match_timeout_seconds: 30.0,
            results_seconds: 0.5,
        });
        for index in 0..script.players {
            game.player(
                &format!("p{index}"),
                Vec3::new(index as f32 * 4.0, 40.0, 0.0),
            );
        }
        game
    }

    /// Run a script, checking [`invariants`] after every tick.
    ///
    /// # Panics
    /// On the first tick whose end state breaks an invariant, naming the tick
    /// and the rule rather than leaving a wrong final state to be worked
    /// backwards from.
    pub fn run(&self, script: &Script) {
        let players = self.players();
        if players.is_empty() {
            return;
        }
        let kits = kit::registry(&self.world);

        for (index, action) in script.actions.iter().enumerate() {
            self.apply(*action, &players, &kits);
            if matches!(action, Action::Tick)
                && let Err(broken) = invariants::check(&self.world)
            {
                panic!("invariant broken after action {index} ({action:?}): {broken}");
            }
        }
    }

    /// Run a script, taking a [`Fingerprint`] after every tick.
    ///
    /// The per-tick series rather than a final hash, so a replay that diverges
    /// reports the tick it diverged on. A single end-of-run comparison says
    /// only that something, somewhere, differed.
    pub fn run_recording(&self, script: &Script) -> Vec<Fingerprint> {
        let players = self.players();
        let kits = kit::registry(&self.world);
        let mut series = Vec::new();
        if players.is_empty() {
            return series;
        }
        for action in &script.actions {
            self.apply(*action, &players, &kits);
            if matches!(action, Action::Tick) {
                series.push(self.fingerprint());
            }
        }
        series
    }

    fn apply(&self, action: Action, players: &[Entity], kits: &[Entity]) {
        let at = |index: usize| self.world.entity_from_id(players[index % players.len()]);

        match action {
            Action::Tick => {
                self.world.progress_time(TICK);
            }
            Action::Hit {
                attacker,
                victim,
                amount,
                kind,
            } => {
                let attacker = at(attacker);
                let victim = at(victim);
                if attacker.id() == victim.id() {
                    return;
                }
                hurt(victim, Damaged {
                    attacker: Some(attacker.id()),
                    amount,
                    knockback: Knockback::from(
                        attacker.try_get::<&Position>(|p| p.0).unwrap_or(Vec3::ZERO),
                    ),
                    kind: Action::kind(kind),
                });
            }
            Action::UseSlot { player, slot } => ability::use_slot(at(player), slot % 9),
            Action::Move { player, to } => {
                at(player).set(Position(to));
            }
            Action::Ground { player, on } => {
                at(player).set(OnGround(on));
            }
            Action::SelectKit { player, kit } => {
                if kits.is_empty() {
                    return;
                }
                let chosen = self.world.entity_from_id(kits[kit % kits.len()]);
                kit::apply(&self.world, at(player), chosen);
            }
        }
    }
}

/// The rules that must hold at the end of every tick.
///
/// Each one is a sentence about the game rather than a restatement of a line of
/// source: "lives never go negative" is checkable against Mineplex's own copy,
/// where "`saturating_sub` was called" is not. That distinction is the whole
/// point -- a check that mirrors the implementation passes for any
/// implementation, including a wrong one.
pub mod invariants {
    use flecs_ecs::prelude::*;
    use smash::module::{
        ability::Cooldown,
        arena::Arena,
        damage::MatchClock,
        lives::{Eliminated, InvulnerableUntil, Lives, MAX_LIVES, Placement, RespawnAt},
        lobby::{Lobby, Phase},
        player::{Energy, Health, Player, Position},
    };

    use super::TICK;

    /// Check every invariant, returning the first that fails.
    ///
    /// # Errors
    /// A sentence naming the rule and the values that broke it.
    pub fn check(world: &World) -> Result<(), String> {
        let lobby = world.cloned::<&Lobby>();
        let arena = world.cloned::<&Arena>();
        let clock = world.cloned::<&MatchClock>().0;

        if lobby.timer < 0.0 {
            return Err(format!(
                "the lobby timer went negative: {:?} at {}",
                lobby.phase, lobby.timer
            ));
        }
        if !clock.is_finite() || clock < 0.0 {
            return Err(format!("the match clock is not a sane time: {clock}"));
        }

        let mut failure = None;
        let mut fail = |message: String| {
            if failure.is_none() {
                failure = Some(message);
            }
        };

        world
            .query::<(&Lives, &Health, &Position)>()
            .with(Player::id())
            .build()
            .each_entity(|player, (lives, health, position)| {
                let name = player.name();

                // Four lives, then you spectate. Never five, and never -1
                // wrapped round to 255 by a subtraction that forgot to
                // saturate.
                if lives.0 > MAX_LIVES {
                    fail(format!("{name} has {} lives, above the maximum", lives.0));
                }

                if !health.current.is_finite() || !health.max.is_finite() {
                    fail(format!("{name} has non-finite health: {health:?}"));
                }
                if health.current < 0.0 {
                    fail(format!("{name} has negative health: {}", health.current));
                }
                if health.current > health.max {
                    fail(format!(
                        "{name} has {} health out of a maximum of {}",
                        health.current, health.max
                    ));
                }

                if !position.0.is_finite() {
                    fail(format!(
                        "{name} is at a non-finite position: {}",
                        position.0
                    ));
                }

                let eliminated = player.has(Eliminated::id());
                if eliminated != (lives.0 == 0) {
                    fail(format!(
                        "{name} has {} lives but is{} eliminated",
                        lives.0,
                        if eliminated { "" } else { " not" }
                    ));
                }
                if eliminated && player.has(RespawnAt::id()) {
                    fail(format!("{name} is eliminated and still queued to respawn"));
                }
                if eliminated != player.try_get::<&Placement>(|p| p.0).is_some() {
                    fail(format!(
                        "{name}'s placement disagrees with their elimination"
                    ));
                }

                if let Some(energy) = player.try_get::<&Energy>(|e| *e)
                    && (energy.current < 0.0 || energy.current > energy.max)
                {
                    fail(format!(
                        "{name} has {} energy out of a maximum of {}",
                        energy.current, energy.max
                    ));
                }

                // A live player is never under the map at the end of a tick.
                // The kill plane is only armed during a match, and a player
                // already queued to respawn or inside their respawn immunity is
                // exempt by design -- the mirror still holds the place they
                // died for a tick or two after the teleport.
                //
                // The immunity is compared against a clock one tick behind the
                // one this check reads. `MatchClock` is advanced by
                // `smash::lobby_tick`, which `LobbyModule` registers after
                // `ArenaModule` registers `smash::death_checks`, so the kill
                // plane sees the clock as it stood at the start of the tick. A
                // player whose immunity runs out mid-tick is therefore killed on
                // the next one, and asserting otherwise would be asserting an
                // ordering the game does not have. Fifty milliseconds of grace
                // is invisible in play; the skew is worth knowing about because
                // *every* reader of `MatchClock` has it, not just this one.
                let armed = matches!(lobby.phase, Phase::Preparing | Phase::Playing);
                let exempt = eliminated
                    || player.has(RespawnAt::id())
                    || player.try_get::<&InvulnerableUntil>(|u| clock - TICK < u.0) == Some(true);
                if armed && !exempt && arena.is_out_of_bounds(position.0) {
                    fail(format!(
                        "{name} is alive at y={} with the kill plane at {}",
                        position.0.y, arena.kill_y
                    ));
                }
            });

        world
            .query::<&Cooldown>()
            .build()
            .each_entity(|ability, cooldown| {
                if cooldown.remaining < 0.0 || !cooldown.remaining.is_finite() {
                    fail(format!(
                        "{} has a cooldown of {}",
                        ability.name(),
                        cooldown.remaining
                    ));
                }
            });

        failure.map_or(Ok(()), Err)
    }
}

/// A fingerprint of everything the game decided.
///
/// Two halves, because they fail differently. The world half catches a
/// simulation that diverged; the call half catches a simulation that reached
/// the same state by telling the clients something different on the way, which
/// is the bug a state comparison misses entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub world: u64,
    pub calls: u64,
}

impl Game {
    /// Hash the whole observable state.
    ///
    /// Floats go in by their bits rather than by a rounded decimal: the
    /// question this answers is whether two runs produced *the same* numbers,
    /// and a tolerance would hide exactly the drift worth finding.
    pub fn fingerprint(&self) -> Fingerprint {
        let mut world = DefaultHasher::new();

        let lobby = self.world.cloned::<&Lobby>();
        format!("{:?}", lobby.phase).hash(&mut world);
        lobby.timer.to_bits().hash(&mut world);
        self.world
            .cloned::<&MatchClock>()
            .0
            .to_bits()
            .hash(&mut world);

        // Sorted by PlayerId, not by entity id: flecs is free to hand out
        // entity ids in a different order and that is not a divergence.
        for player in self.players() {
            let player = self.world.entity_from_id(player);
            player.name().hash(&mut world);
            player.try_get::<&Lives>(|l| l.0).hash(&mut world);
            player.has(Eliminated::id()).hash(&mut world);
            player.try_get::<&Placement>(|p| p.0).hash(&mut world);
            player
                .try_get::<&Health>(|h| (h.current.to_bits(), h.max.to_bits()))
                .hash(&mut world);
            player
                .try_get::<&Position>(|p| p.0.to_array().map(f32::to_bits))
                .hash(&mut world);
            player
                .try_get::<&Velocity>(|v| v.0.to_array().map(f32::to_bits))
                .hash(&mut world);
            player.try_get::<&OnGround>(|g| g.0).hash(&mut world);
            player.try_get::<&JumpsLeft>(|j| j.0).hash(&mut world);
            player
                .try_get::<&Energy>(|e| (e.current.to_bits(), e.max.to_bits()))
                .hash(&mut world);
            player
                .try_get::<&RespawnAt>(|r| r.0.to_bits())
                .hash(&mut world);
            player
                .try_get::<&InvulnerableUntil>(|u| u.0.to_bits())
                .hash(&mut world);

            // Abilities by slot, so the order flecs stores the grants in does
            // not count as a difference.
            #[expect(
                clippy::collection_is_never_read,
                reason = "hashing it is the read; clippy does not count Hash::hash"
            )]
            let mut cooldowns: Vec<(u8, u32)> = Vec::new();
            player.each_target(Grants, |granted| {
                if let (Some(slot), Some(remaining)) = (
                    granted.try_get::<&Slot>(|s| s.0),
                    granted.try_get::<&Cooldown>(|c| c.remaining),
                ) {
                    cooldowns.push((slot, remaining.to_bits()));
                }
            });
            cooldowns.sort_unstable();
            cooldowns.hash(&mut world);
        }

        let mut calls = DefaultHasher::new();
        for call in self.server.calls() {
            format!("{call:?}").hash(&mut calls);
        }

        Fingerprint {
            world: world.finish(),
            calls: calls.finish(),
        }
    }
}
