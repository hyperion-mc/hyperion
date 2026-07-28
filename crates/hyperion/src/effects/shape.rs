//! Where a particle effect puts its particles.
//!
//! Four shapes, because they are what an ability reaches for: a burst marks a
//! point, a line joins two, a ring or disc marks ground, and a sphere marks a
//! volume. Hand-rolling each of these at every call site is how fifteen kits
//! end up with fifteen slightly different circles.
//!
//! Sampling is deterministic. A ring drawn twice from the same arguments has
//! its points in the same places, so a test can assert where they are, and two
//! ticks of a growing ring look like one ring growing rather than like noise.

use glam::Vec3;

/// How far apart consecutive samples of a line are, in blocks, when the caller
/// has not said how many points it wants.
///
/// A quarter of a block: closer than that and a two-block trail costs more
/// packets than it is worth, further and the eye reads it as dots.
pub const DEFAULT_LINE_SPACING: f32 = 0.25;

/// How many samples a ring, disc or sphere takes when the caller has not said.
pub const DEFAULT_POINTS: u32 = 24;

/// The arrangement of points an effect draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// Every particle at one point, scattered only by the effect's offset.
    Burst,
    /// Evenly spaced points from the effect's origin to `to`, both ends
    /// included.
    Line {
        /// The far end.
        to: Vec3,
    },
    /// Points evenly spaced around a circle in the plane through the origin
    /// with the given normal.
    Ring {
        /// Distance from the origin, in blocks.
        radius: f32,
        /// Normal of the plane the circle lies in.
        normal: Vec3,
    },
    /// Points spread over a filled circle rather than its edge.
    Disc {
        /// Radius of the outer edge, in blocks.
        radius: f32,
        /// Normal of the plane the disc lies in.
        normal: Vec3,
    },
    /// Points spread over the surface of a sphere.
    Sphere {
        /// Distance from the origin, in blocks.
        radius: f32,
    },
}

impl Shape {
    /// The radius this shape is drawn at, for the shapes that have one.
    #[must_use]
    pub const fn radius(self) -> Option<f32> {
        match self {
            Self::Burst | Self::Line { .. } => None,
            Self::Ring { radius, .. } | Self::Disc { radius, .. } | Self::Sphere { radius } => {
                Some(radius)
            }
        }
    }

    /// The same shape at a different radius, for the shapes that have one.
    #[must_use]
    pub const fn with_radius(self, radius: f32) -> Self {
        match self {
            Self::Ring { normal, .. } => Self::Ring { radius, normal },
            Self::Disc { normal, .. } => Self::Disc { radius, normal },
            Self::Sphere { .. } => Self::Sphere { radius },
            other => other,
        }
    }
}

/// Two unit vectors spanning the plane with the given normal.
///
/// The seed axis is whichever of x and z the normal leans on least, so the
/// cross product is never taken against a nearly parallel vector; picking a
/// fixed axis makes a ring around that axis collapse to a line.
fn basis(normal: Vec3) -> (Vec3, Vec3) {
    let normal = normal.normalize_or(Vec3::Y);
    let seed = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Z
    };
    let u = normal.cross(seed).normalize_or(Vec3::X);
    (u, normal.cross(u))
}

/// The points a shape puts particles at.
///
/// `points` is a request, not a promise: a burst is always one point, and a
/// line with no explicit count takes as many as [`DEFAULT_LINE_SPACING`] asks
/// for. Nothing here allocates per point; the caller drives the iterator
/// straight into a packet bundle.
#[must_use]
pub fn sample(shape: Shape, origin: Vec3, points: Option<u32>) -> Vec<Vec3> {
    match shape {
        Shape::Burst => vec![origin],
        Shape::Line { to } => {
            let steps = points.unwrap_or_else(|| {
                // At least two, so a line always has both of its ends.
                let span = origin.distance(to);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a span is finite and non-negative, and the max pins the floor"
                )]
                let steps = (span / DEFAULT_LINE_SPACING).ceil() as u32;
                steps.max(1) + 1
            });
            let steps = steps.max(2);
            (0..steps)
                .map(|index| {
                    let t = f32::from(u16::try_from(index).unwrap_or(u16::MAX))
                        / f32::from(u16::try_from(steps - 1).unwrap_or(u16::MAX));
                    origin.lerp(to, t)
                })
                .collect()
        }
        Shape::Ring { radius, normal } => {
            let count = points.unwrap_or(DEFAULT_POINTS).max(1);
            let (u, v) = basis(normal);
            (0..count)
                .map(|index| {
                    let angle = core::f32::consts::TAU * ratio(index, count);
                    origin + (u * angle.cos() + v * angle.sin()) * radius
                })
                .collect()
        }
        Shape::Disc { radius, normal } => {
            let count = points.unwrap_or(DEFAULT_POINTS).max(1);
            let (u, v) = basis(normal);
            (0..count)
                .map(|index| {
                    // A sunflower spiral: the golden angle between successive
                    // points, and a radius growing as the square root of the
                    // index, which is what spreads them evenly over the area
                    // rather than bunching them at the centre.
                    let t = ratio(index, count);
                    let angle = GOLDEN_ANGLE * f32::from(u16::try_from(index).unwrap_or(u16::MAX));
                    let distance = radius * t.sqrt();
                    origin + (u * angle.cos() + v * angle.sin()) * distance
                })
                .collect()
        }
        Shape::Sphere { radius } => {
            let count = points.unwrap_or(DEFAULT_POINTS).max(1);
            (0..count)
                .map(|index| {
                    // A Fibonacci sphere: y stepped uniformly so equal bands
                    // of height hold equal numbers of points, which is what
                    // makes the spacing even on a sphere, and the golden angle
                    // around it so successive points never line up.
                    let y = (-2.0f32).mul_add(ratio(index, count.max(2) - 1), 1.0);
                    let ring = (1.0 - y * y).max(0.0).sqrt();
                    let angle = GOLDEN_ANGLE * f32::from(u16::try_from(index).unwrap_or(u16::MAX));
                    origin + Vec3::new(ring * angle.cos(), y, ring * angle.sin()) * radius
                })
                .collect()
        }
    }
}

/// `index / count`, as a float, without a cast lint at every call.
fn ratio(index: u32, count: u32) -> f32 {
    f32::from(u16::try_from(index).unwrap_or(u16::MAX))
        / f32::from(u16::try_from(count.max(1)).unwrap_or(u16::MAX))
}

/// The angle that never repeats: `TAU / phi^2`.
const GOLDEN_ANGLE: f32 = 2.399_963_2;
