use std::ops::Mul;

use glam::Vec3;

#[derive(Debug, Clone, Copy)]
pub struct Ray {
    origin: Vec3,
    direction: Vec3,
    inv_direction: Vec3,
}

impl Mul<f32> for Ray {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.origin, self.direction * rhs)
    }
}

impl Ray {
    #[must_use]
    pub const fn origin(&self) -> Vec3 {
        self.origin
    }

    #[must_use]
    pub const fn direction(&self) -> Vec3 {
        self.direction
    }

    #[must_use]
    pub const fn inv_direction(&self) -> Vec3 {
        self.inv_direction
    }

    #[must_use]
    #[inline]
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        let inv_direction = direction.map(f32::recip);

        Self {
            origin,
            direction,
            inv_direction,
        }
    }

    #[must_use]
    pub fn from_points(origin: Vec3, end: Vec3) -> Self {
        let direction = end - origin;
        Self::new(origin, direction)
    }

    /// Get the point along the ray at distance t
    #[must_use]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}
