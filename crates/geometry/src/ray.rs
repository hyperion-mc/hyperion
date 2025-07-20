use std::ops::Mul;

use glam::{IVec3, Vec3};

const fn nan_as_inf(value: f32) -> f32 {
    if value.is_nan() { f32::INFINITY } else { value }
}

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
        let inv_direction = direction.map(f32::recip).map(nan_as_inf);

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

    /// Efficiently traverse through grid cells that the ray intersects using the Amanatides and Woo algorithm.
    /// Returns an iterator over the grid cells ([`IVec3`]) that the ray passes through.
    #[inline]
    pub fn voxel_traversal(&self, bounds_min: IVec3, bounds_max: IVec3) -> VoxelTraversal {
        let current_pos = self.origin.as_ivec3();

        // Determine stepping direction for each axis
        let step = IVec3::new(
            if self.direction.x > 0.0 { 1 } else { -1 },
            if self.direction.y > 0.0 { 1 } else { -1 },
            if self.direction.z > 0.0 { 1 } else { -1 },
        );

        // Calculate distance to next voxel boundary for each axis
        let next_boundary = Vec3::new(
            if step.x > 0 {
                current_pos.x as f32 + 1.0 - self.origin.x
            } else {
                self.origin.x - current_pos.x as f32
            },
            if step.y > 0 {
                current_pos.y as f32 + 1.0 - self.origin.y
            } else {
                self.origin.y - current_pos.y as f32
            },
            if step.z > 0 {
                current_pos.z as f32 + 1.0 - self.origin.z
            } else {
                self.origin.z - current_pos.z as f32
            },
        );

        // Calculate t_max and t_delta using precomputed inv_direction
        let t_max = (next_boundary * self.inv_direction.abs()).map(nan_as_inf);
        let t_delta = self.inv_direction.abs();

        VoxelTraversal {
            current_pos,
            step,
            t_max,
            t_delta,
            bounds_min,
            bounds_max,
        }
    }
}

#[derive(Debug)]
#[must_use]
pub struct VoxelTraversal {
    current_pos: IVec3,
    step: IVec3,
    t_max: Vec3,
    t_delta: Vec3,
    bounds_min: IVec3,
    bounds_max: IVec3,
}

impl Iterator for VoxelTraversal {
    type Item = IVec3;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if current position is within bounds
        if self.current_pos.x < self.bounds_min.x
            || self.current_pos.x > self.bounds_max.x
            || self.current_pos.y < self.bounds_min.y
            || self.current_pos.y > self.bounds_max.y
            || self.current_pos.z < self.bounds_min.z
            || self.current_pos.z > self.bounds_max.z
        {
            return None;
        }

        let current = self.current_pos;

        // Determine which axis to step along (the one with minimum t_max)
        if self.t_max.x < self.t_max.y {
            if self.t_max.x < self.t_max.z {
                self.current_pos.x += self.step.x;
                self.t_max.x += self.t_delta.x;
            } else {
                self.current_pos.z += self.step.z;
                self.t_max.z += self.t_delta.z;
            }
        } else if self.t_max.y < self.t_max.z {
            self.current_pos.y += self.step.y;
            self.t_max.y += self.t_delta.y;
        } else {
            self.current_pos.z += self.step.z;
            self.t_max.z += self.t_delta.z;
        }

        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use super::*;

    #[test]
    fn test_traverse_axis_aligned_ray() {
        static DIRECTIONS: [IVec3; 6] = [
            IVec3::new(-1, 0, 0),
            IVec3::new(1, 0, 0),
            IVec3::new(0, -1, 0),
            IVec3::new(0, 1, 0),
            IVec3::new(0, 0, -1),
            IVec3::new(0, 0, 1),
        ];

        static ORIGIN: IVec3 = IVec3::new(-1, 0, 1);

        for direction in DIRECTIONS {
            let ray = Ray::new(ORIGIN.as_vec3(), direction.as_vec3());
            let voxels = ray
                .voxel_traversal(IVec3::MIN, IVec3::MAX)
                .take(10)
                .collect::<Vec<_>>();
            assert_eq!(voxels[0], ORIGIN);
            for (a, b) in voxels.iter().tuple_windows() {
                assert_eq!(b - a, direction);
            }
        }
    }
}
