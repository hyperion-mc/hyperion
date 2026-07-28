//! The map: floating platforms over a lethal nothing.
//!
//! Mineplex's maps have no walls. The kill plane is the map's own configured
//! minimum Y, and water counts as the same thing — the game set
//! `WorldWaterDamage = 1000`, so an ocean under the platforms kills exactly
//! like a void does. Fall damage is switched off entirely, which is why a
//! purely vertical launch is survivable and a horizontal one is not.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    flecs_ext::WorldRefExt,
    module::{
        damage::MatchClock,
        lives::{self, DeathCause, Eliminated, RespawnAt},
        lobby::{Lobby, Phase},
        player::{Health, Player, Position},
    },
};

/// The arena, as a singleton.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Arena {
    pub name: &'static str,
    /// Below this Y a player is dead. Per map, not a global constant.
    pub kill_y: f32,
    /// Where respawns and the opening scatter put players.
    pub spawns: Vec<Vec3>,
}

impl Default for Arena {
    /// The first committed arena, in its own local coordinates.
    ///
    /// This used to be six hand-written coordinates and a kill plane at y 0,
    /// and they described no terrain that has ever existed: nothing placed a
    /// block at any of them, and a player put there would have fallen through
    /// empty air past a death plane thirty blocks below. Reading a real map
    /// file instead is what stops the fallback drifting away from the game
    /// again, because there is no second copy of the numbers to drift.
    ///
    /// `terrain.rs` overwrites this with the same map offset into its region
    /// before anyone connects, so on a running server the default is only ever
    /// the value between two `world.set` calls. It matters for the tests under
    /// `tests/`, which import the game without a host and get their arena from
    /// here.
    fn default() -> Self {
        let spec = crate::map::parse(crate::map::ARENAS[0]).expect(
            "the first committed arena parses; `tests/maps.rs` proves every one of them does",
        );
        Self {
            name: spec.name,
            kill_y: spec.kill_y,
            spawns: spec.spawns,
        }
    }
}

impl Arena {
    /// Spawn point `index`, wrapping. Deterministic so a test can predict it.
    #[must_use]
    pub fn spawn(&self, index: u64) -> Vec3 {
        if self.spawns.is_empty() {
            return Vec3::new(0.0, 64.0, 0.0);
        }
        let len = self.spawns.len() as u64;
        self.spawns[usize::try_from(index % len).unwrap_or(0)]
    }

    #[must_use]
    pub fn is_out_of_bounds(&self, at: Vec3) -> bool {
        at.y < self.kill_y
    }
}

#[derive(Component)]
pub struct ArenaModule;

impl Module for ArenaModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Arena");
        world.component::<Arena>().add_trait::<flecs::Singleton>();
        world.set(Arena::default());

        // Both death checks read `Health`, and killing someone writes it from
        // the `Died` observer. flecs catches that overlap at runtime, so the
        // victims are collected first and killed once the query has finished.
        // Health reaching zero is the rarer path but a real one: hunger and
        // lava both get there.
        world
            .system_named::<()>("death_checks")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    // The arena is only lethal while a match is running.
                    // Mineplex got this for free because its hub was a separate
                    // world; hyperion serves one set of chunks, so the hub is a
                    // region of the same world and the kill plane would
                    // otherwise reach it. Without this gate a player standing in
                    // the lobby with a kill plane above them dies on the tick
                    // they connect, four times, and is eliminated before the
                    // game starts.
                    if !matches!(
                        world.cloned::<&Lobby>().phase,
                        Phase::Preparing | Phase::Playing
                    ) {
                        continue;
                    }
                    let arena = world.cloned::<&Arena>();
                    let clock = world.cloned::<&MatchClock>().0;
                    let mut doomed = Vec::new();

                    world
                        .query::<(&Position, &Health)>()
                        .with(Player::id())
                        .without(Eliminated::id())
                        .without(RespawnAt::id())
                        .build()
                        .each_entity(|player, (position, health)| {
                            if lives::is_invulnerable(player, clock) {
                                return;
                            }
                            if health.is_dead() {
                                doomed.push((player.id(), DeathCause::Damage));
                            } else if arena.is_out_of_bounds(position.0) {
                                doomed.push((player.id(), DeathCause::Void));
                            }
                        });

                    for (player, cause) in doomed {
                        lives::kill(world.entity_at(player), cause);
                    }
                }
            });
    }
}
