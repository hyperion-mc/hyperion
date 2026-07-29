//! Where an entity is, which way it faces, and how big it is.
//!
//! Split out because these are the components nearly every other system
//! reads and almost none of them write: position, rotation, bounding size,
//! chunk membership, velocity, and the conversions between them. Nothing here
//! knows about packets or about the ECS schedule.

use derive_more::{Constructor, Deref, DerefMut, From};
use flecs_ecs::prelude::*;
use geometry::aabb::Aabb;
use glam::{I16Vec2, IVec3, Vec3};
use serde::{Deserialize, Serialize};

/// The full pose of an entity. This is used for both [`super::Player`] and
/// [`super::Npc`].
#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    Deref,
    DerefMut,
    From,
    PartialEq
)]
#[flecs(meta)]
pub struct Position {
    /// The (x, y, z) position of the entity.
    /// Note we are using [`Vec3`] instead of [`glam::DVec3`] because *cache locality* is important.
    /// However, the Notchian server uses double precision floating point numbers for the position.
    position: Vec3,
}

impl Position {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: Vec3::new(x, y, z),
        }
    }

    /// The position in the eighths-of-a-block units sound packets use.
    #[must_use]
    pub fn sound_position(&self) -> IVec3 {
        let position = self.position * 8.0;
        position.as_ivec3()
    }

    /// Get the chunk position of the center of the player's bounding box.
    ///
    /// Chunk coordinates are `i16` on the wire (`SetChunkCacheCenter` and
    /// friends), so this is a narrowing conversion. It is total by clamping
    /// rather than panicking: it runs every tick for every player on a
    /// position ultimately derived from client input, and one out-of-range
    /// coordinate used to abort the whole process here via
    /// `try_from().unwrap()`. The move handler already rejects positions
    /// outside the world border (see
    /// `handlers::change_position_or_correct_client`); this clamp is
    /// the belt-and-braces that keeps the conversion itself incapable of
    /// crashing whoever calls it.
    #[must_use]
    #[expect(clippy::cast_possible_truncation)]
    pub fn to_chunk(&self) -> I16Vec2 {
        // `as i32` on a float saturates (and maps NaN to 0), so a huge or
        // infinite coordinate lands at i32::MIN/MAX rather than wrapping; the
        // clamp then brings it into the representable i16 chunk range.
        let to_chunk_axis = |coord: f32| -> i16 {
            let block = (coord as i32) >> 4;
            block.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        };

        I16Vec2::new(to_chunk_axis(self.x), to_chunk_axis(self.z))
    }
}

#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Deref,
    DerefMut,
    Default,
    Constructor,
    PartialEq
)]
#[flecs(meta)]
pub struct Yaw {
    yaw: f32,
}

impl std::fmt::Display for Yaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let yaw = self.yaw;
        write!(f, "{yaw}")
    }
}

#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Deref,
    DerefMut,
    Default,
    Constructor,
    PartialEq
)]
#[flecs(meta)]
pub struct Pitch {
    pitch: f32,
}

impl std::fmt::Display for Pitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pitch = self.pitch;
        write!(f, "{pitch}")
    }
}

const PLAYER_WIDTH: f32 = 0.6;
const PLAYER_HEIGHT: f32 = 1.8;

// No #[flecs(meta)]: `super::register_reflection` registers this as an opaque
// serialised through Display, and flecs aborts if a type is registered as both
// a struct and an opaque. `super::Uuid` is the same shape.
#[derive(Component, Copy, Clone, Debug, Constructor, PartialEq)]
pub struct EntitySize {
    pub half_width: f32,
    pub height: f32,
}

impl core::fmt::Display for EntitySize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let half_width = self.half_width;
        let height = self.height;
        write!(f, "{half_width}x{height}")
    }
}

impl Default for EntitySize {
    fn default() -> Self {
        Self {
            half_width: PLAYER_WIDTH / 2.0,
            height: PLAYER_HEIGHT,
        }
    }
}

#[derive(Component, Debug, Copy, Clone)]
#[flecs(meta)]
pub struct ChunkPosition {
    pub position: I16Vec2,
}

const SANE_MAX_RADIUS: i16 = 128;

impl ChunkPosition {
    #[must_use]
    #[expect(missing_docs)]
    pub const fn null() -> Self {
        // todo: huh
        Self {
            position: I16Vec2::new(SANE_MAX_RADIUS, SANE_MAX_RADIUS),
        }
    }
}

#[must_use]
pub fn aabb(position: Vec3, size: EntitySize) -> Aabb {
    let half_width = size.half_width;
    let height = size.height;
    Aabb::new(
        position - Vec3::new(half_width, 0.0, half_width),
        position + Vec3::new(half_width, height, half_width),
    )
}

#[must_use]
pub fn block_bounds(position: Vec3, size: EntitySize) -> (IVec3, IVec3) {
    let bounding = aabb(position, size);
    let min = bounding.min.floor().as_ivec3();
    let max = bounding.max.ceil().as_ivec3();

    (min, max)
}

/// The initial player spawn position. todo: this should not be a constant
pub const PLAYER_SPAWN_POSITION: Vec3 = Vec3::new(-8_526_209_f32, 100f32, -6_028_464f32);

/// The reaction of an entity, in particular to collisions as calculated in `entity_detect_collisions`.
///
/// Why is this useful?
///
/// - We want to be able to detect collisions in parallel.
/// - Since we are accessing bounding boxes in parallel,
///   we need to be able to make sure the bounding boxes are immutable (unless we have something like a
///   [`std::sync::Arc`] or [`std::sync::RwLock`], but this is not efficient).
/// - Therefore, we have an [`Velocity`] component which is used to store the reaction of an entity to collisions.
/// - Later we can apply the reaction to the entity's [`Position`] to move the entity.
#[derive(Component, Default, Debug, Copy, Clone, PartialEq)]
#[flecs(meta)]
pub struct Velocity(pub Vec3);

impl Velocity {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self(Vec3::new(x, y, z))
    }

    #[must_use]
    pub fn to_packet_units(self) -> valence_protocol::Velocity {
        valence_protocol::Velocity::from_ms_f32((self.0 * 20.0).into())
    }
}

#[derive(Component, Default, Debug, Copy, Clone, PartialEq)]
pub struct PendingTeleportation {
    pub teleport_id: i32,
    pub destination: Vec3,
    pub ttl: u8,
}

impl PendingTeleportation {
    #[must_use]
    pub fn new(destination: Vec3) -> Self {
        Self {
            teleport_id: fastrand::i32(..),
            destination,
            ttl: 20,
        }
    }
}

#[must_use]
pub fn get_direction_from_rotation(yaw: f32, pitch: f32) -> Vec3 {
    // Convert angles from degrees to radians
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();

    Vec3::new(
        -pitch_rad.cos() * yaw_rad.sin(), // x = -cos(pitch) * sin(yaw)
        -pitch_rad.sin(),                 // y = -sin(pitch)
        pitch_rad.cos() * yaw_rad.cos(),  // z = cos(pitch) * cos(yaw)
    )
}

/// Registration module for the core kinematics base: where an entity is
/// ([`Position`]) and how it is moving ([`Velocity`]).
///
/// This is the "settled base" of the flecs module convention (see the root
/// `CLAUDE.md`): the two components nearly every system reads, registered in
/// one importable place so nothing re-registers them ad hoc. Both the
/// simulation ([`super::SimComponentsModule`]) and the projectile physics
/// import this rather than each calling `world.component::<Position>()`
/// themselves, and a use-before-register of the kinematics base becomes an
/// import edge the DAG supplies rather than an ordering a contributor can miss
/// (ENG-11000's class).
///
/// It imports [`super::ReflectionComponentsModule`] because `Position` and
/// `Velocity` register their reflection through the glam `Vec3` meta that module
/// installs, so a standalone `world.import::<KinematicsComponentsModule>()` into
/// a bare world is self-sufficient. Registration only, no behavior.
#[derive(Component)]
pub struct KinematicsComponentsModule;

impl Module for KinematicsComponentsModule {
    fn module(world: &World) {
        // `Position`/`Velocity` carry `#[flecs(meta)]` over a `Vec3`, so the
        // glam reflection has to exist before their meta is registered.
        world.import::<super::ReflectionComponentsModule>();
        world.component::<Position>().meta();
        world.component::<Velocity>().meta();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: a position past the i16 chunk range (|coord| > 32767 * 16)
    // used to make `to_chunk`'s `i16::try_from(..).unwrap()` panic, and it runs
    // every tick for every player from a client-supplied position, so one
    // packet took down the whole process. It must now be total.
    #[test]
    fn to_chunk_is_total_for_out_of_border_positions() {
        for coord in [
            1.0e9_f32,
            -1.0e9,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            29_999_984.0,
            -29_999_984.0,
        ] {
            let chunk = Position::new(coord, 100.0, coord).to_chunk();
            // The whole point is that it returned rather than panicking; the
            // clamped value is representable by construction.
            let _ = chunk;
        }
    }
}
