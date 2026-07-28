//! Whether a player may fly, how fast, and what fell from where.
//!
//! [`abilities`] is the one place that turns these components into the packet
//! the client is told them through.

use flecs_ecs::prelude::*;
use glam::{DVec3, Vec3};
use hyperion_minecraft_proto::packets::play::player::{AbilityFlags, PlayerAbilities};

#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct FlyingSpeed {
    pub speed: f32,
}

impl FlyingSpeed {
    #[must_use]
    pub const fn new(speed: f32) -> Self {
        Self { speed }
    }
}

impl Default for FlyingSpeed {
    fn default() -> Self {
        Self { speed: 0.05 }
    }
}

#[derive(Component, Default, Debug, Copy, Clone)]
pub struct MovementTracking {
    pub fall_start_y: f32,
    pub last_tick_flying: bool,
    pub last_tick_position: Vec3,
    pub received_movement_packets: u8,
    pub server_velocity: DVec3,
    pub sprinting: bool,
    pub was_on_ground: bool,
}

#[derive(Component, Default, Debug, Copy, Clone)]
#[flecs(meta)]
pub struct Flight {
    pub allow: bool,
    pub is_flying: bool,
}

/// What the client should be told it may do, given the two components that say
/// so.
///
/// Flight permission and flying speed live in separate components that are set
/// independently, and the packet carries both, so either one changing has to
/// resend the pair.
pub(crate) const fn abilities(flight: Flight, flying_speed: FlyingSpeed) -> PlayerAbilities {
    let mut flags = AbilityFlags::NONE;
    if flight.allow {
        flags = flags.union(AbilityFlags::CAN_FLY);
    }
    if flight.is_flying {
        flags = flags.union(AbilityFlags::FLYING);
    }

    PlayerAbilities {
        flags,
        flying_speed: flying_speed.speed,
        // Zero is what hyperion has always put in this slot, back when valence
        // called it `fov_modifier`. Vanilla sends 0.1. ENG-10456 tracks
        // whether that difference is visible.
        walking_speed: 0.0,
    }
}
