//! Projectiles are entities, integrated by one system.
//!
//! Arrows, hooks, sulphur bombs and thrown blocks differ only in their numbers
//! and in what they do on contact, so they are one component set and one
//! function pointer rather than four ability implementations each with their own
//! flight loop.
//!
//! There is no block collision here: that needs the host's world, which is on
//! the far side of the seam. Projectiles expire on a timer and on entity
//! contact. `docs/smash-design.md` lists this as one of the two places the
//! simulation is deliberately incomplete pending the hyperion wiring.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    flecs_ext::WorldRefExt,
    module::{
        damage::{DamageKind, Damaged, hurt},
        knockback::Knockback,
        player::{Health, Player, Position},
        sound,
    },
    server::{PlayerId, ServerHandle, Sound, SoundCategory},
};

/// Tag on projectile entities.
#[derive(Component, Debug)]
pub struct Projectile;

#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Flight {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Blocks per second squared, downwards. Arrows have it, hooks do not.
    pub gravity: f32,
    pub seconds_left: f32,
    /// Anything within this of the projectile counts as hit.
    pub radius: f32,
}

/// Who fired it, so a projectile cannot hit its owner and the kill is credited.
#[derive(Component, Debug)]
pub struct FiredBy;

/// What a projectile does to whoever it touches.
#[derive(Component, Debug, Copy, Clone)]
pub struct Payload {
    pub damage: f32,
    pub knockback: f32,
    /// Runs in addition to the damage. Iron Hook uses it to reel the victim in;
    /// most projectiles leave it as a no-op.
    pub on_hit: fn(&Impact<'_>),
}

/// Handed to [`Payload::on_hit`].
pub struct Impact<'a> {
    pub world: WorldRef<'a>,
    pub projectile: EntityView<'a>,
    pub shooter: Option<EntityView<'a>>,
    pub victim: EntityView<'a>,
    pub at: Vec3,
}

pub const fn no_extra_effect(_: &Impact<'_>) {}

impl Payload {
    #[must_use]
    pub const fn new(damage: f32, knockback: f32) -> Self {
        Self {
            damage,
            knockback,
            on_hit: no_extra_effect,
        }
    }

    #[must_use]
    pub const fn then(mut self, on_hit: fn(&Impact<'_>)) -> Self {
        self.on_hit = on_hit;
        self
    }
}

/// Fire one.
///
/// Does not hand the projectile back: every caller so far sets everything it
/// needs through [`Flight`] and [`Payload`], and returning an entity nobody
/// uses only invites someone to hold it past the tick it dies on.
pub fn fire(world: WorldRef<'_>, shooter: EntityView<'_>, flight: Flight, payload: Payload) {
    world
        .new_entity()
        .add(Projectile::id())
        .set(flight)
        .set(payload)
        .add((FiredBy, shooter));
}

#[derive(Component)]
pub struct ProjectileModule;

impl Module for ProjectileModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Projectile");

        world.component::<Projectile>();
        world.component::<Flight>();
        world.component::<Payload>();
        world.component::<FiredBy>().add(flecs::Exclusive);

        world
            .system_named::<(&mut Flight, &Payload)>("smash::fly")
            .each_iter(|it, index, (flight, payload)| {
                let dt = it.delta_time();
                let projectile = it.entity(index);
                let from = flight.position;
                flight.velocity.y = flight.gravity.mul_add(-dt, flight.velocity.y);
                flight.position += flight.velocity * dt;
                flight.seconds_left -= dt;

                if flight.seconds_left <= 0.0 {
                    projectile.destruct();
                    return;
                }

                let shooter = projectile.target(FiredBy, 0).map(|e| e.id());
                let Some((victim, at)) = nearest_target(
                    projectile.world(),
                    from,
                    flight.position,
                    flight.radius,
                    shooter,
                ) else {
                    return;
                };

                let world = projectile.world();
                let victim = world.entity_at(victim);
                hurt(victim, Damaged {
                    attacker: shooter,
                    amount: payload.damage,
                    knockback: Knockback::from(at).times(payload.knockback),
                    kind: DamageKind::Projectile,
                });

                (payload.on_hit)(&Impact {
                    world,
                    projectile,
                    shooter: shooter.map(|s| world.entity_at(s)),
                    victim,
                    at,
                });

                world.get::<&ServerHandle>(|server| {
                    // At the crossing point, not at either endpoint of the
                    // step and not at the victim: a projectile that connects
                    // mid-step connected somewhere neither of those is.
                    server.play_sound(
                        at,
                        Sound::new(sound::PROJECTILE_HIT, SoundCategory::Players),
                    );
                    // And the shooter gets told, however far away they are.
                    if let Some(id) =
                        shooter.and_then(|s| world.entity_at(s).try_get::<&PlayerId>(|p| *p))
                    {
                        server.play_sound_to(
                            id,
                            Sound::new(sound::RANGED_HITMARKER, SoundCategory::Players),
                        );
                    }
                });
                projectile.destruct();
            });
    }
}

/// The point on the segment `from`..`to` nearest `point`.
fn closest_on_segment(from: Vec3, to: Vec3, point: Vec3) -> Vec3 {
    let along = to - from;
    let length_squared = along.length_squared();
    if length_squared <= f32::EPSILON {
        return from;
    }
    from + along * ((point - from).dot(along) / length_squared).clamp(0.0, 1.0)
}

/// Closest living player within `radius` of the segment this tick swept,
/// excluding the shooter, and the point on that segment where it connected.
///
/// A segment and not a point. Integration moves a projectile a whole tick's
/// travel at once, and Barrage's arrows travel at 60 blocks per second, which
/// at 20 ticks is three blocks a step against a hit radius of 0.4: sampling
/// only the endpoint means seven eighths of the flight path is a hole a player
/// can stand in. Every arrow ability in the game was decided by whether a
/// victim happened to be standing on a sample point.
fn nearest_target(
    world: WorldRef<'_>,
    from: Vec3,
    to: Vec3,
    radius: f32,
    exclude: Option<Entity>,
) -> Option<(Entity, Vec3)> {
    let mut best: Option<(f32, Entity, Vec3)> = None;
    world
        .query::<(&Position, &Health)>()
        .with(Player::id())
        .build()
        .each_entity(|entity, (position, health)| {
            if health.is_dead() || Some(entity.id()) == exclude {
                return;
            }
            let at = closest_on_segment(from, to, position.0);
            let distance = position.0.distance(at);
            if distance > radius {
                return;
            }
            if best.is_none_or(|(closest, ..)| distance < closest) {
                best = Some((distance, entity.id(), at));
            }
        });
    best.map(|(_, entity, at)| (entity, at))
}
