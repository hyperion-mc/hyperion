//! Projectiles are entities, integrated by one system.
//!
//! Arrows, hooks, sulphur bombs and thrown blocks differ only in their numbers
//! and in what they do on contact, so they are one component set and one
//! function pointer rather than four ability implementations each with their own
//! flight loop.
//!
//! Block collision is a swept segment against [`crate::module::blocks`], the
//! read seam onto the host's terrain, and it is checked before the entity
//! search so a player standing behind a wall cannot be shot through it.
//! A projectile that meets a block stops on its surface, sticks there for
//! [`STUCK_SECONDS`], and expires. A world with no terrain seam installed --
//! every test that is not about terrain, and the whole of the mock -- answers
//! "clear" and the flight is exactly what it was before.
//!
//! What the flight above is authoritative for is the *hit*. What a client sees
//! is drawn separately, by `crate::draw` on the host, off the [`Visual`] this
//! module carries -- so the game half stays testable against the mock with no
//! host anywhere near it, and the same `Flight` that decides the damage decides
//! where the picture is. See [`fire`].

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::{entity_kind::EntityKind, projectile_motion::EYE_HEIGHT};

use crate::{
    flecs_ext::WorldRefExt,
    module::{
        blocks::{BlockWorldComponentsModule, BlockWorldHandle},
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

/// A projectile that has hit a block and is embedded in it.
///
/// A tag rather than a zeroed velocity, because "stopped" and "stopped by
/// something" need to be different states. `fly` excludes stuck projectiles
/// outright: a projectile whose sweep starts on the face it just hit hits that
/// face again every tick, which would replay the impact sound forever, and one
/// sitting still is a point a player can walk into and be shot by a projectile
/// that is no longer going anywhere. Neither is a check to add inside `fly` --
/// they are the same statement, that a stuck projectile is done moving and done
/// hitting, and a query term says it once.
#[derive(Component, Debug)]
pub struct Stuck;

/// How long a projectile stays visible in the block it hit.
///
/// Vanilla leaves an arrow in a wall for a minute. This is a fighting game
/// played in a small arena: long enough to read the impact as an impact, short
/// enough that a Barrage volley does not leave the far wall bristling for the
/// rest of the match.
pub const STUCK_SECONDS: f32 = 1.0;

/// What a projectile is drawn as.
///
/// A generated [`EntityKind`] and not a stand-in enum a table maps, the same
/// choice #1035 made for particles: an ability names the vanilla thing it wants
/// to look like and the host draws exactly that. The game half carries it as a
/// plain field -- `crate::draw` on the host is what turns it into a spawned
/// entity -- so a test world that never imports a host still compiles and runs,
/// it just draws nothing.
///
/// Exact where a vanilla entity is the thing (an arrow is [`EntityKind::Arrow`],
/// an egg [`EntityKind::Egg`]); the closest always-rendered projectile where the
/// real thing has no entity of its own (a thrown coal, an ink pellet), marked
/// `[APPROXIMATED]` at the call site.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Visual(pub EntityKind);

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

/// Fire one, drawn as `visual`.
///
/// Does not hand the projectile back: every caller sets everything it needs
/// through [`Flight`], [`Payload`] and [`Visual`], and returning an entity
/// nobody uses only invites someone to hold it past the tick it dies on.
pub fn fire(
    world: WorldRef<'_>,
    shooter: EntityView<'_>,
    visual: Visual,
    mut flight: Flight,
    payload: Payload,
) {
    // Launch from the shooter's eye, not the tracked feet: the flight the hit
    // is computed on is then the flight the client is shown (`crate::draw`
    // renders `flight.position` directly), so a shot that visually passes
    // through a player is a shot that hits one. `nearest_target` treats the
    // victim as an upright body so an eye-high shot still connects.
    flight.position += Vec3::Y * EYE_HEIGHT;
    world
        .new_entity()
        .add(Projectile::id())
        .set(visual)
        .set(flight)
        .set(payload)
        .add((FiredBy, shooter));
}

/// Registration module for projectiles: the component set and nothing else.
///
/// Split from [`ProjectileModule`] per the root `CLAUDE.md`. A consumer that
/// wants the *types* -- `crate::draw` on the host decorates a projectile it
/// never integrates -- imports this without dragging in the flight systems.
///
/// Prefixed, and it has to be. flecs registers a module entity under its bare
/// Rust type name before the body that renames it runs, so two crates in one
/// world cannot each have a `ProjectileComponentsModule` -- and
/// `hyperion::simulation::projectile_motion` already does. The collision is an
/// `ecs_assert`, which means a dev build aborts on boot and a release build,
/// where flecs asserts are compiled out, silently treats the two modules as one
/// (ENG-12054). This name is what keeps that from happening; do not shorten it.
#[derive(Component)]
pub struct SmashProjectileComponentsModule;

impl Module for SmashProjectileComponentsModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Projectile");

        // Final: a projectile is a leaf, never an inheritance base.
        world.component::<Projectile>().add_trait::<flecs::Final>();
        world.component::<Stuck>().add_trait::<flecs::Final>();
        world.component::<Visual>();
        world.component::<Flight>();
        world.component::<Payload>();
        // Relationship, so `(FiredBy, shooter)` can never be a bare tag.
        // Exclusive, because a projectile has one shooter. `DontFragment`,
        // because the target is a player and a fragmenting relationship would
        // mint an archetype per shooter as volleys fly. `(OnDeleteTarget,
        // Delete)`: a projectile dies with the player who fired it, so a
        // shooter disconnecting mid-flight takes their arrows with them rather
        // than leaving them to land crediting nobody.
        world
            .component::<FiredBy>()
            .add_trait::<flecs::Relationship>()
            .add(flecs::Exclusive)
            .add_trait::<flecs::DontFragment>()
            .add_trait::<(flecs::OnDeleteTarget, flecs::Delete)>();
    }
}

/// Behavior module for projectiles: integration, block collision and the hit.
#[derive(Component)]
pub struct ProjectileModule;

impl Module for ProjectileModule {
    fn module(world: &World) {
        // Imports before the scope claim, and that order is load bearing.
        // `world.module` creates `smash::Projectile` the first time and
        // thereafter only sets the scope to it, so whichever of the two modules
        // runs first owns the path. Letting the registration module own it is
        // what keeps `Flight` and friends at `smash.Projectile.Flight` whether
        // they were reached through this module or imported on their own.
        world.import::<SmashProjectileComponentsModule>();
        // `fly` reads the terrain seam's singleton, so the module that
        // registers it is imported here rather than assumed. A full boot
        // happens to register it first; a standalone import of this module
        // would not, and the difference between the two is invisible in a
        // release build.
        world.import::<BlockWorldComponentsModule>();
        world.module::<Self>("smash::Projectile");

        world
            .system_named::<(&mut Flight, &Payload)>("fly")
            .without(Stuck::id())
            .each_iter(|it, index, (flight, payload)| {
                let dt = it.delta_time();
                let projectile = it.entity(index);
                let world = projectile.world();
                let from = flight.position;
                flight.velocity.y = flight.gravity.mul_add(-dt, flight.velocity.y);
                let mut to = flight.velocity.mul_add(Vec3::splat(dt), from);
                flight.seconds_left -= dt;

                if flight.seconds_left <= 0.0 {
                    projectile.destruct();
                    return;
                }

                // The terrain sweep before the entity search, and the search
                // then runs over the *clipped* segment. That ordering is the
                // whole of "you cannot be shot through a wall": a player
                // standing behind one is not on the segment the arrow got to
                // travel, so `nearest_target` never sees them.
                let impact = world.get::<&BlockWorldHandle>(|blocks| blocks.sweep(world, from, to));
                if let Some(impact) = impact {
                    to = impact.point;
                }
                flight.position = to;

                let shooter = projectile.target(FiredBy, 0).map(|e| e.id());
                let Some((victim, at)) = nearest_target(world, from, to, flight.radius, shooter)
                else {
                    if let Some(impact) = impact {
                        // Stopped on the surface it met, and left there to be
                        // seen. `Flight` is what `crate::draw` reads in
                        // `PostUpdate`, so writing the impact point above is
                        // what puts the picture in the wall rather than a
                        // tick's travel past it.
                        flight.velocity = Vec3::ZERO;
                        flight.gravity = 0.0;
                        flight.seconds_left = flight.seconds_left.min(STUCK_SECONDS);
                        projectile.add(Stuck::id());
                        world.get::<&ServerHandle>(|server| {
                            // `Neutral`, not `Players`: an arrow striking
                            // terrain is a thing that happened in the world,
                            // and vanilla's own `AbstractArrow` puts it on that
                            // slider. The hit on a player stays on `Players`.
                            server.play_sound(
                                impact.point,
                                Sound::new(sound::PROJECTILE_HIT, SoundCategory::Neutral),
                            );
                        });
                    }
                    return;
                };

                let victim = world.entity_at(victim);
                hurt(victim, Damaged {
                    attacker: shooter,
                    amount: payload.damage,
                    knockback: Knockback::from(at - flight.velocity).times(payload.knockback),
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

        // Stuck projectiles are excluded from `fly` entirely, so something has
        // to run their clock. A second system and not a branch inside `fly`,
        // because the two states share nothing: this one neither moves nor
        // hits, it only counts down.
        world
            .system_named::<&mut Flight>("expire_stuck")
            .with(Stuck::id())
            .each_iter(|it, index, flight| {
                flight.seconds_left -= it.delta_time();
                if flight.seconds_left <= 0.0 {
                    it.entity(index).destruct();
                }
            });
    }
}

/// A standing player's height in blocks. The hit treats a victim as an upright
/// segment from the feet to here, not a point at the feet: an arrow crosses a
/// player at any height, and now that projectiles launch from the eye the shot
/// travels at chest height, not ankle height. Matching only the feet is what
/// let an eye-high shot sail over a target standing in front of the shooter.
const PLAYER_HEIGHT: f32 = 1.8;

/// The nearest pair of points between two segments, one on each, clamped to the
/// endpoints (Ericson, *Real-Time Collision Detection*). Used to measure the
/// arrow's swept path against the victim's upright body.
#[expect(
    clippy::many_single_char_names,
    reason = "the single-letter names are Ericson's own notation for the \
              closest-points-between-segments algorithm; prose names would obscure the \
              correspondence to the reference"
)]
fn closest_between_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);

    if a <= f32::EPSILON && e <= f32::EPSILON {
        return (p1, p2);
    }

    let (s, t) = if a <= f32::EPSILON {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= f32::EPSILON {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a.mul_add(e, -(b * b));
            let s = if denom > f32::EPSILON {
                (b.mul_add(f, -(c * e)) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t = b.mul_add(s, f) / e;
            if t < 0.0 {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else if t > 1.0 {
                (((b - c) / a).clamp(0.0, 1.0), 1.0)
            } else {
                (s, t)
            }
        }
    };

    (p1 + d1 * s, p2 + d2 * t)
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
            let (at, on_body) = closest_between_segments(
                from,
                to,
                position.0,
                position.0 + Vec3::Y * PLAYER_HEIGHT,
            );
            let distance = at.distance(on_body);
            if distance > radius {
                return;
            }
            if best.is_none_or(|(closest, ..)| distance < closest) {
                best = Some((distance, entity.id(), at));
            }
        });
    best.map(|(_, entity, at)| (entity, at))
}
