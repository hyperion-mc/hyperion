//! The map: floating platforms over a lethal nothing.
//!
//! Mineplex's maps have no walls. The kill plane is the map's own configured
//! minimum Y, and water counts as the same thing — the game set
//! `WorldWaterDamage = 1000`, so an ocean under the platforms kills exactly
//! like a void does. Fall damage is switched off entirely, which is why a
//! purely vertical launch is survivable and a horizontal one is not.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::module::{
    lives::{self, DeathCause},
    player::{Health, Position},
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
    fn default() -> Self {
        Self {
            name: "Skylands",
            kill_y: 0.0,
            spawns: vec![
                Vec3::new(-12.0, 34.0, 0.0),
                Vec3::new(12.0, 34.0, 0.0),
                Vec3::new(0.0, 34.0, -12.0),
                Vec3::new(0.0, 34.0, 12.0),
                Vec3::new(-8.0, 40.0, -8.0),
                Vec3::new(8.0, 40.0, 8.0),
            ],
        }
    }
}

impl Arena {
    /// Spawn point `index`, wrapping. Deterministic so a test can predict it.
    #[must_use]
    pub fn spawn(&self, index: usize) -> Vec3 {
        if self.spawns.is_empty() {
            return Vec3::new(0.0, 64.0, 0.0);
        }
        self.spawns[index % self.spawns.len()]
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

        world
            .system_named::<(&Position, &Health, &Arena)>("smash::void_check")
            .each_entity(|player, (position, health, arena)| {
                if health.is_dead() || !arena.is_out_of_bounds(position.0) {
                    return;
                }
                lives::kill(player, DeathCause::Void);
            });

        // Health reaching zero is rare but real: hunger and lava both do it.
        world
            .system_named::<&Health>("smash::zero_health_check")
            .each_entity(|player, health| {
                if health.is_dead() {
                    lives::kill(player, DeathCause::Damage);
                }
            });
    }
}
