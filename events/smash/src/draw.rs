//! The host half of a projectile: the part a client can see.
//!
//! A smash projectile is a game-half entity carrying [`Flight`], [`Payload`] and
//! [`Visual`]. `Flight` is authoritative for the hit and is integrated by
//! `smash::fly` with no host anywhere near it, which is what keeps
//! `tests/abilities.rs` and the mock able to prove a projectile connects. None
//! of that reaches a client, because nothing tells one the projectile exists.
//!
//! This module is that telling, and only that. On the host it decorates each new
//! projectile with the components hyperion's entity egress reads -- a kind, a
//! position, a velocity, a facing -- and enqueues the spawn. The client then
//! dead-reckons the arc from the velocity the `add_entity` packet carries, which
//! is how vanilla draws a thrown projectile between server updates.
//!
//! Two consequences worth stating plainly:
//!
//! It is a separate entity from nothing -- it *is* the projectile, decorated.
//! So when `smash::fly` destructs the projectile on its hit or its timer, the
//! entity that was drawn is the entity that dies, and hyperion's
//! `despawn_removed_entity` observer tells every client to drop it. There is no
//! second lifetime to keep in step.
//!
//! The drawn arc and the hit are computed twice, by two integrators that do not
//! share constants: hyperion's client-side dead reckoning uses the vanilla
//! gravity for the entity kind, and `Flight` uses whatever the ability set.
//! They agree exactly for a zero-gravity projectile -- a hook, a line of cows --
//! and drift for a heavy one over its flight. The projectile vanishes at the
//! hit either way, because the hit is what destructs it. Making the two share
//! one integrator is the larger change flagged in `docs/smash-design.md`; this
//! is the visible-now half.
//!
//! Deliberately not [`hyperion::effects::spawn::spawn`], which creates a *new*
//! entity: decorating the existing one is the whole reason despawn is free. And
//! deliberately no [`hyperion::simulation::Owner`], which is what
//! `update_projectile_positions` requires to integrate and collide an entity --
//! omitting it is what stops hyperion from moving or re-hitting a projectile
//! `Flight` already owns.

use flecs_ecs::prelude::*;
use hyperion::{
    net::Channel,
    simulation::{
        BroadcastProjectile, Pitch, Position, Spawn, Uuid, Velocity, Yaw,
        projectile_motion::look_angles,
    },
};

use crate::module::projectile::{Flight, Projectile, Visual};

/// Marks a projectile that has already been drawn, so it is decorated once.
#[derive(Component, Debug)]
struct Drawn;

/// Minecraft runs at twenty ticks a second, and hyperion's [`Velocity`] is in
/// blocks per tick where [`Flight`] is in blocks per second. Every crossing
/// between the two goes through this.
pub(crate) const TICKS_PER_SECOND: f32 = 20.0;

#[derive(Component)]
pub struct DrawModule;

impl Module for DrawModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Draw");
        world.component::<Drawn>();

        // A system rather than an `OnAdd` observer, because `fire` adds the
        // `Projectile` tag before it sets `Flight`, so an observer on the tag
        // would see the projectile a step before it has a position. A system
        // that matches on all three and skips what it has already drawn sees a
        // whole projectile or none of it, and picks a mid-tick spawn up on the
        // next tick -- one tick, fifty milliseconds, invisible against a flight
        // that lasts hundreds.
        world
            .system_named::<(&Visual, &Flight)>("smash::draw_projectiles")
            .with(Projectile::id())
            .without(Drawn::id())
            .each_entity(|projectile, (visual, flight)| {
                let per_tick = flight.velocity / TICKS_PER_SECOND;
                let (yaw, pitch) = look_angles(flight.velocity);
                // Render the flight position directly: `fire` already launched
                // the projectile from the shooter's eye, so the picture and the
                // hit are the same flight. One source, no visual/sim offset.
                projectile
                    .add_enum(visual.0)
                    .set(Uuid::new_v4())
                    .set(Position::new(
                        flight.position.x,
                        flight.position.y,
                        flight.position.z,
                    ))
                    .set(Velocity::new(per_tick.x, per_tick.y, per_tick.z))
                    .set(Yaw::new(yaw))
                    .set(Pitch::new(pitch))
                    // A broadcast channel of its own and the marker that says
                    // to keep sending its position: advance_drawn_projectiles
                    // below moves the wire components each tick and hyperion's
                    // broadcast_marked_projectiles sends them, so the client
                    // sees the arc rather than dead-reckoning from the spawn.
                    // No Owner, so hyperion never integrates or re-hits it;
                    // Flight stays the one authority for where it goes.
                    .add(Channel)
                    .add(BroadcastProjectile)
                    .add(Drawn::id());
                // Enqueued rather than added: the `Spawn` observer that sends
                // `add_entity` runs at the sync point, after this system's
                // deferred component adds above have been applied, so the packet
                // it builds sees the kind and position this set.
                projectile.enqueue(Spawn);
            });

        // Move the wire components each tick from the `Flight` that owns the
        // motion, so hyperion's `broadcast_marked_projectiles` has a fresh
        // position to send. `PostUpdate`, after `smash::fly` has integrated
        // `Flight` this tick. The projectile points where it is going, re-aimed
        // off its velocity every tick, the same as a hyperion-owned arrow.
        world
            .system_named::<(&Flight, &mut Position, &mut Velocity, &mut Yaw, &mut Pitch)>(
                "smash::advance_drawn_projectiles",
            )
            .with(Projectile::id())
            .with(Drawn::id())
            .kind(id::<flecs::pipeline::PostUpdate>())
            .each(|(flight, position, velocity, yaw, pitch)| {
                let (y, p) = look_angles(flight.velocity);
                **position = flight.position;
                velocity.0 = flight.velocity / TICKS_PER_SECOND;
                **yaw = y;
                **pitch = p;
            });
    }
}
