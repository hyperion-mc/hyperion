//! What an ability looks and feels like: particles, motion, area and spawning.
//!
//! These are engine capabilities rather than game concepts. Nothing here knows
//! what a kit is; it knows how to draw a ring, how to push a player away from
//! a point, who is standing inside a radius, and how to put a new entity in
//! the world. A game built on hyperion composes those into an ability.
//!
//! Sound is deliberately absent. [`crate::net::agnostic::sound`] already plays
//! one at an arbitrary point in the world, attenuated by distance, and a
//! second path to the same packet is worth less than the one that exists.
//!
//! Only [particles](particle) is a flecs module, because only particles have
//! state that outlives the call that started them. Motion, area queries,
//! spawning and status effects are functions over components other modules
//! already own, or over packets the client itself keeps state for, and
//! wrapping each in a `Module` that registers nothing would be ceremony.

pub mod area;
pub mod motion;
pub mod particle;
pub mod shape;
pub mod spawn;
pub mod status;

use flecs_ecs::prelude::*;

pub use self::{
    area::{Hit, nearest_player, players_within},
    motion::{apply_impulse, knock_back, knockback_impulse, quantized, set_velocity},
    particle::{Effect, ParticleEmitter, ParticleModule},
    shape::Shape,
    spawn::{launch, spawn},
    status::{Status, clear as clear_status},
};

/// Everything an ability needs to be seen and felt.
#[derive(Component)]
pub struct EffectsModule;

impl Module for EffectsModule {
    fn module(world: &World) {
        world.module::<Self>("hyperion::Effects");
        world.import::<ParticleModule>();
        world.import::<self::spawn::SpawnModule>();
    }
}
