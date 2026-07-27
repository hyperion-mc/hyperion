//! Player state that every other module reads.
//!
//! Position, rotation and ground state are *mirrors*: the adapter writes them
//! from the host server once per tick and nothing in the game writes them back.
//! Keeping them as components rather than trait calls is what lets the arena
//! bounds check and the cooldown tick stay pure component iteration.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{flecs_ext::EntityViewExt, server::PlayerId};

/// Tag: this entity is a player taking part in the game.
#[derive(Component, Debug, Default)]
pub struct Player;

/// Mirror of the host's position. Written by the adapter, read by the game.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Position(pub Vec3);

/// Mirror of the host's velocity.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Velocity(pub Vec3);

/// Unit vector the player is looking along. Abilities that fire "where you look"
/// read this and nothing else.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Facing(pub Vec3);

impl Default for Facing {
    fn default() -> Self {
        Self(Vec3::NEG_Z)
    }
}

/// Mirror of the host's ground state. Gates double jump and the abilities that
/// Mineplex required you to be grounded for (Fissure, Seismic Slam).
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct OnGround(pub bool);

/// Health in half-hearts out of [`Health::max`].
///
/// This is the quantity that drives knockback: in Mineplex SSM a low health bar
/// means you fly further, which is the inverse of Smash Bros' accumulating
/// damage percentage but produces the same feel.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    #[must_use]
    pub const fn full(max: f32) -> Self {
        Self { current: max, max }
    }

    /// 1.0 at full health, 0.0 at death. The single input the knockback model
    /// takes from the victim's condition.
    #[must_use]
    pub fn fraction(self) -> f32 {
        if self.max <= 0.0 {
            return 0.0;
        }
        (self.current / self.max).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn is_dead(self) -> bool {
        self.current <= 0.0
    }

    pub fn damage(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }

    pub fn heal(&mut self, amount: f32) {
        self.current = (self.current + amount).min(self.max);
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::full(20.0)
    }
}

/// A charge or ammo pool. Only kits that have one carry it, which is why it is
/// a separate component rather than a field on [`Health`].
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Energy {
    pub current: f32,
    pub max: f32,
    /// Units per second.
    pub regen: f32,
}

impl Energy {
    #[must_use]
    pub const fn full(max: f32, regen: f32) -> Self {
        Self {
            current: max,
            max,
            regen,
        }
    }

    /// Spend `amount` if it is there. Returns whether it was.
    pub fn try_spend(&mut self, amount: f32) -> bool {
        if self.current + f32::EPSILON < amount {
            return false;
        }
        self.current -= amount;
        true
    }
}

/// How many double jumps remain before touching ground again.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct JumpsLeft(pub u8);

/// Registers the mirrored components and the systems that maintain them.
#[derive(Component)]
pub struct PlayerModule;

impl Module for PlayerModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Player");

        world.component::<Player>();
        world.component::<Position>();
        world.component::<Velocity>();
        world.component::<Facing>();
        world.component::<OnGround>();
        world.component::<Health>();
        world.component::<Energy>();
        world.component::<JumpsLeft>();

        // A player is never meaningfully without these, and forgetting one is
        // the kind of bug that shows up as a silently non-matching query three
        // modules away. `With` makes flecs add them for us.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Position)>()
            .add_trait::<(flecs::With, Velocity)>()
            .add_trait::<(flecs::With, Facing)>()
            .add_trait::<(flecs::With, OnGround)>()
            .add_trait::<(flecs::With, Health)>()
            .add_trait::<(flecs::With, JumpsLeft)>();

        world
            .system_named::<(&OnGround, &mut JumpsLeft)>("smash::restore_double_jump")
            .each(|(ground, jumps)| {
                if ground.0 {
                    jumps.0 = 1;
                }
            });

        world
            .system_named::<&mut Energy>("smash::regen_energy")
            .each_iter(|it, _, energy| {
                energy.current = (energy.regen.mul_add(it.delta_time(), energy.current))
                    .min(energy.max);
            });
    }
}

/// Send a game event to one player.
///
/// Every event in the game is *about a player*, and flecs matches an observer
/// when the emitted id matches any of its terms. Tagging every event with
/// [`Player`] and filtering every observer on [`Player`] makes that one rule
/// instead of a per-event convention nobody can check.
pub fn notify<E: ComponentId>(player: EntityView<'_>, event: &E) {
    player.emit_about::<Player, _>(event);
}

/// Convenience for tests and for the adapter: make a player entity.
pub fn spawn_player<'w>(world: &'w World, id: PlayerId, name: &str) -> EntityView<'w> {
    world.entity_named(name).set(id).add(Player::id())
}
