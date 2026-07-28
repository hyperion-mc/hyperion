//! The map's edges, and what happens when a player crosses one.
//!
//! Every playable region is a box a player is meant to stay inside, and every
//! box carries a [`Policy`] for the moment somebody leaves it. There are two,
//! and they are opposites: the arena eliminates you (falling off is how the
//! game is won and lost), and the hub shoves you back in (a lobby you can jump
//! out of is a lobby, briefly, with nobody in it).
//!
//! That is the whole of it, and it is why there is no longer a phase check
//! gating a kill plane. The gate existed only because the hub shares one world
//! with the arena, so the arena's floor reached up into the lobby and had to be
//! switched off there. Expressed as "which bounds is this player in, and what
//! is its policy", the lobby case stops being an exception and becomes the
//! other policy: the phase selects the region everyone currently occupies --
//! the game moves them all together, so it is one region at a time -- and the
//! region's own policy decides what its edge does.
//!
//! Mineplex's maps have no walls; the kill plane is the map's configured
//! minimum Y, and water is the same thing (`WorldWaterDamage = 1000`). Fall
//! damage is off, which is why a vertical launch is survivable and a horizontal
//! one is not.

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
    server::{Particle, Particles, PlayerId, ServerHandle, Sound, SoundCategory},
};

/// What the edge of a region does to a player who crosses it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Policy {
    /// Shove them back inside with velocity. The hub. Never lethal.
    PushBack,
    /// Take a life. The arena, where leaving the floor is how the game is
    /// played.
    Eliminate,
}

/// A region a player is meant to stay inside, and what its edge does.
///
/// An axis-aligned box, because "nearest point on the boundary" -- which is
/// what a push has to aim back through -- is one `clamp` for a box and a
/// projection-per-face for anything else, and no arena needs the difference.
/// The arena's box is a floor and nothing else: infinite horizontally, open at
/// the top, bounded only below at `kill_y`, because the only edge an arena has
/// is the one you fall through.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
    pub policy: Policy,
}

impl Bounds {
    /// Whether `at` is inside.
    #[must_use]
    pub fn contains(&self, at: Vec3) -> bool {
        at.cmpge(self.min).all() && at.cmple(self.max).all()
    }

    /// A unit vector pointing from `at` back to the nearest point on the box,
    /// or `None` when `at` is already inside.
    ///
    /// The nearest-point normal and not a direction toward the centre: a player
    /// who jumps out over one wall should be shoved straight back through that
    /// wall, not sent on a diagonal toward the middle that a non-square arena
    /// would make land somewhere they were not. `clamp` gives the nearest point
    /// on the box for free, and the vector to it is that normal.
    #[must_use]
    pub fn back_inside(&self, at: Vec3) -> Option<Vec3> {
        let nearest = at.clamp(self.min, self.max);
        let toward = nearest - at;
        (toward.length_squared() > f32::EPSILON).then(|| toward.normalize())
    }
}

/// The hub's bounds, as a singleton.
///
/// Static -- there is one hub -- and read from `map::HUB` at boot rather than
/// from a hand-written constant, for the same reason [`Arena::default`] reads a
/// real map: there is then no second copy of the numbers to drift from the
/// geometry a player actually stands on. The hub is region zero at the world
/// origin, so its declared bounds are already world coordinates and need no
/// offset; an arena's would, which is one more reason the arena carries its
/// edge as a `kill_y` the terrain offsets rather than as one of these.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct HubBounds(pub Bounds);

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

    /// The arena as a [`Bounds`]: a floor at `kill_y` and nothing else.
    ///
    /// Everything above the floor and anywhere horizontally is inside, so
    /// [`Bounds::contains`] is false exactly when a player has fallen below the
    /// kill plane -- which is what [`is_out_of_bounds`](Self::is_out_of_bounds)
    /// meant, now stated as the general thing every region is.
    #[must_use]
    pub const fn bounds(&self) -> Bounds {
        Bounds {
            min: Vec3::new(f32::NEG_INFINITY, self.kill_y, f32::NEG_INFINITY),
            max: Vec3::splat(f32::INFINITY),
            policy: Policy::Eliminate,
        }
    }

    #[must_use]
    pub fn is_out_of_bounds(&self, at: Vec3) -> bool {
        at.y < self.kill_y
    }
}

/// The bounds every player is currently inside, or `None` when nothing is
/// enforced.
///
/// The phase is the honest name for which region players occupy, because the
/// game moves them all together: they are in the hub through `Waiting` and
/// `Countdown`, scattered onto the arena at `Preparing`, and left on it through
/// `Playing`. `Ended` is the one phase with players in the arena and no edge
/// enforced -- the match is decided, the results screen is up, and a player who
/// slid off in the last instant is about to be teleported home rather than
/// eliminated a second time.
#[must_use]
fn governing(phase: Phase, hub: Option<HubBounds>, arena: &Arena) -> Option<Bounds> {
    match phase {
        Phase::Waiting | Phase::Countdown => hub.map(|hub| hub.0),
        Phase::Preparing | Phase::Playing => Some(arena.bounds()),
        Phase::Ended => None,
    }
}

/// How hard the hub shoves a player back over its wall, in blocks per tick.
///
/// Applied straight to velocity and deliberately **not** through the knockback
/// model's `1 + 0.1 * missing` health term: that term throws a nearly-dead
/// player further, and a wall that throws a nearly-dead player further is a
/// wall that kills them -- which turns a fence into a hazard and the lobby into
/// something you can die in. A fixed shove is a fence.
///
/// Firmer than a double jump (0.42) so it always wins against the launch that
/// carried the player out, and short of an ability's launch so it reads as a
/// bump and not a catapult.
const PUSH_IMPULSE: f32 = 0.9;

/// The shove's sound. A shield's block is the closest vanilla has to the thud
/// of hitting a wall you cannot pass.
const PUSH_SOUND: &str = "minecraft:item.shield.block";

#[derive(Component)]
pub struct ArenaModule;

impl Module for ArenaModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Arena");
        world.component::<Arena>().add_trait::<flecs::Singleton>();
        world.component::<Bounds>();
        world
            .component::<HubBounds>()
            .add_trait::<flecs::Singleton>();
        world.set(Arena::default());

        // The hub's own bounds, read from the map it is built from. Parsed here
        // rather than handed over by `terrain.rs` so the game half has it with
        // no host -- `tests/` exercise the push against the mock -- and because
        // the hub is region zero at the origin, the map's coordinates are
        // already world ones.
        let hub = crate::map::parse(crate::map::HUB)
            .expect("the hub map parses; `tests/maps.rs` proves it")
            .bounds
            .expect("the hub map declares `bounds`; without it the lobby has no walls");
        world.set(HubBounds(Bounds {
            min: hub.0,
            max: hub.1,
            policy: Policy::PushBack,
        }));

        // One system for both edges. It reads which region the phase puts
        // players in, then applies that region's policy -- eliminate at the
        // arena floor, shove at the hub wall -- so the lobby and the kill plane
        // are the same mechanism with opposite policies rather than one
        // mechanism and a special case.
        //
        // Collected-then-applied for the reason it always was: killing a player
        // and shoving one both write components a query is reading (`Health`,
        // and the velocity the server pushes), and flecs refuses that from
        // inside the query that found them.
        world.system_named::<()>("bounds_checks").run(|mut it| {
            while it.next() {
                let world = it.world();
                let Some(bounds) = governing(
                    world.cloned::<&Lobby>().phase,
                    world.try_cloned::<&HubBounds>(),
                    &world.cloned::<&Arena>(),
                ) else {
                    continue;
                };
                let clock = world.cloned::<&MatchClock>().0;

                let mut doomed = Vec::new();
                let mut shoved = Vec::new();

                world
                    .query::<(&Position, &Health, &PlayerId)>()
                    .with(Player::id())
                    .without(Eliminated::id())
                    .without(RespawnAt::id())
                    .build()
                    .each_entity(|player, (position, health, id)| {
                        if lives::is_invulnerable(player, clock) {
                            return;
                        }
                        match bounds.policy {
                            Policy::Eliminate => {
                                if health.is_dead() {
                                    doomed.push((player.id(), DeathCause::Damage));
                                } else if !bounds.contains(position.0) {
                                    doomed.push((player.id(), DeathCause::Void));
                                }
                            }
                            Policy::PushBack => {
                                if let Some(normal) = bounds.back_inside(position.0) {
                                    shoved.push((*id, position.0, normal));
                                }
                            }
                        }
                    });

                for (player, cause) in doomed {
                    lives::kill(world.entity_at(player), cause);
                }

                if !shoved.is_empty() {
                    world.get::<&ServerHandle>(|server| {
                        for (id, at, normal) in shoved {
                            server.add_velocity(id, normal * PUSH_IMPULSE);
                            // The shove is a thing that happened to the player,
                            // so it is seen and heard where it happened: the
                            // sparks say "you hit something" and the thud says
                            // it was solid, which is the difference between a
                            // wall and the game rubber-banding a rejected move.
                            server.particles(
                                Particles::burst(Particle::Crit, at)
                                    .count(12)
                                    .offset(Vec3::splat(0.3))
                                    .speed(0.1),
                            );
                            server.play_sound(at, Sound::new(PUSH_SOUND, SoundCategory::Players));
                        }
                    });
                }
            }
        });
    }
}
