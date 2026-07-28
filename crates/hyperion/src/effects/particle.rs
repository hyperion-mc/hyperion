//! Placing particles in the world.
//!
//! The unit is an [`Effect`]: a particle, a shape, and how densely to draw it.
//! One expression builds it and one call sends it, so an ability says what it
//! wants to look like rather than filling in eleven `LevelParticles` fields.
//!
//! An effect that lasts more than a tick is an entity rather than a loop.
//! [`Effect::expanding`] spawns one carrying a [`ParticleEmitter`], and
//! `hyperion::draw_particle_emitters` draws and ages it until it is done. That
//! is the difference between a shockwave that grows while the game runs and
//! one that is drawn at eleven radii inside a single tick.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::chunk::LevelParticles,
    particle::Particle,
};
use tracing::warn;

use crate::{
    effects::shape::{Shape, sample},
    net::{Compose, DataBundle, protocol::Clientbound},
    simulation::Position,
};

/// A particle effect, ready to be placed in the world.
///
/// Built by one of the shape constructors and adjusted by the setters, all of
/// which consume and return, so the whole thing is one expression.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Effect<'a> {
    particle: Particle<'a>,
    origin: Vec3,
    shape: Shape,
    points: Option<u32>,
    count: i32,
    offset: Vec3,
    speed: f32,
    long_distance: bool,
}

impl<'a> Effect<'a> {
    /// The shared body of the shape constructors.
    ///
    /// Written out rather than spelled `..Self::burst(..)`, because struct
    /// update syntax drops the value it took the rest of the fields from, and
    /// a drop is not something a `const fn` may do.
    const fn new(particle: Particle<'a>, origin: Vec3, shape: Shape, count: i32) -> Self {
        Self {
            particle,
            origin,
            shape,
            points: None,
            count,
            offset: Vec3::ZERO,
            speed: 0.0,
            long_distance: false,
        }
    }

    /// Every particle at one point.
    pub const fn burst(particle: Particle<'a>, at: Vec3) -> Self {
        // One packet's worth of particles at one point reads as a puff rather
        // than as a single speck, which is what asking for a burst means.
        Self::new(particle, at, Shape::Burst, 8)
    }

    /// A trail of particles from `from` to `to`.
    pub const fn line(particle: Particle<'a>, from: Vec3, to: Vec3) -> Self {
        // A shape that is already many points draws one particle at each,
        // unless the caller thickens it.
        Self::new(particle, from, Shape::Line { to }, 1)
    }

    /// A circle of particles around `center`, lying flat unless
    /// [`normal`](Self::normal) says otherwise.
    pub const fn ring(particle: Particle<'a>, center: Vec3, radius: f32) -> Self {
        let shape = Shape::Ring {
            radius,
            normal: Vec3::Y,
        };
        Self::new(particle, center, shape, 1)
    }

    /// A filled circle of particles around `center`.
    pub const fn disc(particle: Particle<'a>, center: Vec3, radius: f32) -> Self {
        let shape = Shape::Disc {
            radius,
            normal: Vec3::Y,
        };
        Self::new(particle, center, shape, 1)
    }

    /// A shell of particles around `center`.
    pub const fn sphere(particle: Particle<'a>, center: Vec3, radius: f32) -> Self {
        Self::new(particle, center, Shape::Sphere { radius }, 1)
    }

    /// How many particles to draw at each of the shape's points.
    pub const fn count(mut self, count: i32) -> Self {
        self.count = count;
        self
    }

    /// Half-width of the box each point's particles are scattered through.
    ///
    /// This is the client's own randomisation, so it costs nothing: one packet
    /// with an offset draws a cloud, where the same cloud sampled server-side
    /// would be one packet per particle.
    pub const fn offset(mut self, spread: Vec3) -> Self {
        self.offset = spread;
        self
    }

    /// How fast each particle starts moving, in the client's own units.
    ///
    /// Zero leaves the particle where it was drawn, which is what a shape
    /// meant to be read as a shape wants.
    pub const fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// How many points to sample the shape at.
    ///
    /// Ignored by a burst, which is one point by definition.
    pub const fn points(mut self, points: u32) -> Self {
        self.points = Some(points);
        self
    }

    /// The plane a ring or disc lies in. Ignored by the other shapes.
    pub const fn normal(mut self, normal: Vec3) -> Self {
        self.shape = match self.shape {
            Shape::Ring { radius, .. } => Shape::Ring { radius, normal },
            Shape::Disc { radius, .. } => Shape::Disc { radius, normal },
            other => other,
        };
        self
    }

    /// Draw this past the radius the client normally culls particles at.
    ///
    /// `ClientboundLevelParticlesPacket.overrideLimiter`. Worth setting for
    /// something a spectator further out still wants to see, and not worth it
    /// for ambient decoration.
    pub const fn long_distance(mut self, long_distance: bool) -> Self {
        self.long_distance = long_distance;
        self
    }

    /// Where this effect is centred.
    #[must_use]
    pub const fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The shape this effect draws.
    #[must_use]
    pub const fn shape(&self) -> Shape {
        self.shape
    }

    /// The packets this effect sends, one per sampled point.
    ///
    /// Public so a test can assert on the wire form without a running server,
    /// which is the only way to check that a count, an offset and a particle's
    /// extra data all arrive as the caller asked for them.
    pub fn packets(&self) -> impl Iterator<Item = LevelParticles<'_>> {
        let particle = &self.particle;
        let (count, offset, speed, long_distance) =
            (self.count, self.offset, self.speed, self.long_distance);
        sample(self.shape, self.origin, self.points)
            .into_iter()
            .map(move |at| LevelParticles {
                override_limiter: long_distance,
                // The client's particle setting is the player's own choice, so
                // an effect does not override it.
                always_show: false,
                x: f64::from(at.x),
                y: f64::from(at.y),
                z: f64::from(at.z),
                x_dist: offset.x,
                y_dist: offset.y,
                z_dist: offset.z,
                max_speed: speed,
                count,
                particle: particle.clone(),
            })
    }

    /// Draw this once, now.
    ///
    /// Every point goes out as one bundle to the players near the origin, so a
    /// twenty-four point ring is one broadcast rather than twenty-four.
    pub fn emit(&self, world: WorldRef<'_>) {
        world.get::<&Compose>(|compose| {
            let mut bundle = DataBundle::new(compose);
            for packet in self.packets() {
                if let Err(error) =
                    bundle.add_packet(Clientbound::new(PacketId::LevelParticles.to_raw(), &packet))
                {
                    warn!("dropping a particle effect: {error}");
                    return;
                }
            }
            let center = Position::new(self.origin.x, self.origin.y, self.origin.z).to_chunk();
            if let Err(error) = bundle.broadcast_local(center) {
                warn!("dropping a particle effect: {error}");
            }
        });
    }
}

impl Effect<'static> {
    /// Draw this every tick for `seconds`, growing to `to` blocks.
    ///
    /// Only meaningful for a shape with a radius; a burst or a line has
    /// nothing to grow, and expanding one draws it repeatedly in place.
    ///
    /// Returns the entity carrying the effect, so a caller that wants to stop
    /// it early can destruct it. It removes itself when its time is up.
    #[must_use]
    pub fn expanding(self, to: f32, seconds: f32) -> ParticleEmitter {
        ParticleEmitter {
            from: self.shape.radius().unwrap_or_default(),
            to,
            duration: seconds,
            elapsed: 0.0,
            effect: self,
        }
    }
}

/// A particle effect that is still being drawn.
///
/// A component rather than a loop inside whoever started it: the effect
/// outlives the ability that cast it, and an ability that owned the loop could
/// not end without either cutting the effect short or blocking.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct ParticleEmitter {
    /// What to draw, at whatever radius the emitter has reached.
    pub effect: Effect<'static>,
    /// Radius at the start, in blocks.
    pub from: f32,
    /// Radius when the time is up, in blocks.
    pub to: f32,
    /// How long the whole sweep takes, in seconds.
    pub duration: f32,
    /// How much of it has already run, in seconds.
    pub elapsed: f32,
}

impl ParticleEmitter {
    /// Put this in the world, where the drawing system will pick it up.
    #[must_use = "the entity is the handle for stopping the effect early"]
    pub fn emit(self, world: WorldRef<'_>) -> Entity {
        world.entity().set(self).id()
    }

    /// The effect as it looks right now.
    pub fn current(&self) -> Effect<'static> {
        let t = if self.duration > 0.0 {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let radius = self.to.mul_add(t, self.from * (1.0 - t));
        let mut effect = self.effect.clone();
        effect.shape = effect.shape.with_radius(radius);
        effect
    }
}

/// Particles: the [`Effect`] surface, and the system that ages a
/// [`ParticleEmitter`].
#[derive(Component)]
pub struct ParticleModule;

impl Module for ParticleModule {
    fn module(world: &World) {
        world.module::<Self>("hyperion::Particle");
        world.component::<ParticleEmitter>();

        world
            .system_named::<&mut ParticleEmitter>("draw_particle_emitters")
            .each_iter(|it, index, emitter| {
                let entity = it.entity(index);
                emitter.current().emit(it.world());
                emitter.elapsed += it.delta_time();
                if emitter.elapsed >= emitter.duration {
                    // Destructing inside the iteration is why this is deferred:
                    // flecs holds the write until the system finishes, so the
                    // table this row lives in is not restructured mid-sweep.
                    entity.world().defer(|| entity.destruct());
                }
            });
    }
}
