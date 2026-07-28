//! Putting a new entity in the world.
//!
//! The [`Spawn`] tag and the observer behind it already do the work: give an
//! entity a kind, a position and a rotation, enqueue `Spawn`, and every client
//! nearby is sent an `add_entity`. What was missing is that the observer needs
//! five components present and says nothing useful when one is absent, so
//! every caller wrote the same five `set`s and one of them eventually wrote
//! four.
//!
//! [`spawn`] is those five in one call. It hands the entity back rather than
//! finishing the job, because what a projectile, a dropped item and a
//! decoration need after that have nothing in common.

use flecs_ecs::{
    core::{Entity, EntityView, WorldRef, flecs, id},
    prelude::*,
};
use glam::Vec3;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::entity::RemoveEntities,
};
use hyperion_utils::EntityExt;
use tracing::error;

use crate::{
    net::{Compose, ConnectionId, protocol::Clientbound},
    simulation::{Owner, Pitch, Position, Spawn, Uuid, Velocity, Yaw, entity_kind::EntityKind},
};

/// Put an entity of `kind` into the world at `at`.
///
/// Sets everything the `Spawn` observer reads and enqueues the tag, so the
/// packet goes out at the end of the current phase rather than in the middle
/// of whatever is iterating. Chain further components onto the return value;
/// they land before the spawn does.
///
/// ```ignore
/// let arrow = spawn(world, EntityKind::Arrow, muzzle)
///     .set(Velocity::new(aim.x, aim.y, aim.z))
///     .set(Owner::new(shooter));
/// ```
#[must_use]
pub fn spawn(world: WorldRef<'_>, kind: EntityKind, at: Vec3) -> EntityView<'_> {
    // `WorldRef::entity` returns a view borrowing the `WorldRef` rather than
    // the world, so returning one straight from here does not compile. The id
    // is a plain integer; re-resolving it against the world threads the right
    // lifetime through. Same fix as `smash::flecs_ext::WorldRefExt::new_entity`,
    // which should move here once this crate has a second caller for it.
    let entity = EntityView::new_from(world, world.entity().id());
    entity
        .add_enum(kind)
        .set(Uuid::new_v4())
        .set(Position::new(at.x, at.y, at.z))
        .set(Velocity::default())
        .set(Yaw::default())
        .set(Pitch::default());
    // Enqueued rather than added, so the observer that sends `add_entity` runs
    // after the caller has finished attaching whatever else this entity needs.
    // `enqueue` returns nothing, so it cannot be part of the chain above.
    entity.enqueue(Spawn);
    entity
}

/// Put a projectile of `kind` into the world, moving at `velocity`.
///
/// Same as [`spawn`] plus the two things every projectile has: the velocity it
/// was launched with, and who launched it. [`Owner`] is what
/// `update_projectile_positions` reads to stop an arrow colliding with the bow
/// that fired it, and what a kill is credited through.
///
/// The facing is derived from the velocity rather than taken from the shooter,
/// because an arrow points where it is going. A projectile launched with no
/// velocity keeps the default facing rather than picking one from a zero
/// vector.
#[must_use]
pub fn launch(
    world: WorldRef<'_>,
    kind: EntityKind,
    at: Vec3,
    velocity: Vec3,
    shooter: Entity,
) -> EntityView<'_> {
    let entity = spawn(world, kind, at)
        .set(Velocity::new(velocity.x, velocity.y, velocity.z))
        .set(Owner::new(shooter));
    if velocity == Vec3::ZERO {
        return entity;
    }
    let (yaw, pitch) = facing(velocity);
    entity.set(Yaw::new(yaw)).set(Pitch::new(pitch))
}

/// The yaw and pitch, in degrees, of something travelling along `direction`.
///
/// Minecraft's yaw is zero facing south (+z) and increases towards west, which
/// is why this is `atan2(-x, z)` rather than the `atan2(z, x)` the maths would
/// suggest.
#[must_use]
pub fn facing(direction: Vec3) -> (f32, f32) {
    let horizontal = direction.x.hypot(direction.z);
    let yaw = (-direction.x).atan2(direction.z).to_degrees();
    let pitch = (-direction.y).atan2(horizontal).to_degrees();
    (yaw, pitch)
}

/// How much longer a temporary entity stays in the world, in seconds.
///
/// A component rather than a timer held by whoever spawned the thing: a
/// turret outlives the ability that placed it, and an ability holding the
/// timer could not finish without either cutting the turret short or blocking.
///
/// Counted down on flecs' own delta time and not on any game clock, so an
/// entity spawned in a lobby ages at the same rate as one spawned in a match.
/// A clock that only runs during play would freeze a hub decoration forever.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Lifetime {
    /// Seconds left. At or below zero the entity is destructed.
    pub seconds: f32,
}

impl Lifetime {
    /// A lifetime of `seconds`.
    #[must_use]
    pub const fn new(seconds: f32) -> Self {
        Self { seconds }
    }
}

/// Spawning, despawning, and the lifetime that connects them.
#[derive(Component)]
pub struct SpawnModule;

impl Module for SpawnModule {
    fn module(world: &World) {
        world.module::<Self>("hyperion::Spawn");
        world.component::<Lifetime>();

        world
            .system_named::<&mut Lifetime>("hyperion::expire_temporary_entities")
            .each_iter(|it, index, lifetime| {
                lifetime.seconds -= it.delta_time();
                if lifetime.seconds <= 0.0 {
                    let entity = it.entity(index);
                    // Deferred, because destructing during iteration would
                    // restructure the table this row is being read out of.
                    entity.world().defer(|| entity.destruct());
                }
            });

        // Tell every client to drop an entity this server dropped.
        //
        // `remove_player_from_visibility` in the ingress module already does
        // this, but only for something in `PacketState::Play`, which is to say
        // only for a player. Everything else -- a projectile, a dropped item,
        // a turret -- was spawned with `add_entity` and, until this observer,
        // was never unspawned: the server forgot it and every client kept
        // drawing it where it died, forever.
        world
            .observer::<flecs::OnRemove, ()>()
            .with_enum_wildcard::<EntityKind>()
            .without(id::<ConnectionId>())
            .each_entity(|entity, ()| {
                let packet = RemoveEntities(vec![entity.id().minecraft_id()]);
                entity.world().get::<&Compose>(|compose| {
                    if let Err(error) = compose
                        .broadcast(Clientbound::new(PacketId::RemoveEntities.to_raw(), &packet))
                        .send()
                    {
                        error!("failed to unspawn an entity: {error}");
                    }
                });
            });
    }
}
