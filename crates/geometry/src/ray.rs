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

    /// Efficiently traverse through grid cells that the ray intersects using the Amanatides and Woo algorithm.
    /// Returns an iterator over the grid cells ([`IVec3`]) that the ray passes through.
    ///
    /// Unbounded in `t`: it runs until it leaves the box. Anything asking about
    /// a finite length of ray -- one tick of a projectile, a player's reach --
    /// wants [`Self::voxel_traversal_until`] instead, which is the difference
    /// between walking four cells and walking to the edge of the coordinate
    /// space.
    #[inline]
    pub fn voxel_traversal(&self, bounds_min: IVec3, bounds_max: IVec3) -> VoxelTraversal {
        self.voxel_traversal_until(bounds_min, bounds_max, f32::INFINITY)
    }

    /// The same traversal, stopping once the ray has travelled `max_t`.
    ///
    /// `t` is in units of [`Self::direction`], so a ray built with
    /// [`Self::from_points`] is bounded to the segment by `max_t == 1.0`.
    ///
    /// The bound is a real cost, not a tidiness: an axis-aligned ray in an
    /// empty world visits every cell out to the coordinate bound, and the only
    /// reason that was survivable was that it usually found *something*.
    #[inline]
    pub fn voxel_traversal_until(
        &self,
        bounds_min: IVec3,
        bounds_max: IVec3,
        max_t: f32,
    ) -> VoxelTraversal {
        // `floor`, not `as_ivec3`. The cast truncates towards zero, which is
        // the same thing for a positive coordinate and one cell out for a
        // negative one -- so every traversal that began at a negative
        // coordinate began in the wrong cell, and half of any Minecraft map is
        // at a negative coordinate.
        let current_pos = self.origin.floor().as_ivec3();

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
            t_entry: 0.0,
            max_t,
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
    /// How far along the ray the current cell was entered.
    t_entry: f32,
    /// The last `t` worth reporting a cell for.
    max_t: f32,
}

impl Iterator for VoxelTraversal {
    type Item = IVec3;

    fn next(&mut self) -> Option<Self::Item> {
        // Past the end of the ray the caller asked about.
        if self.t_entry > self.max_t {
            return None;
        }

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

        // Determine which axis to step along (the one with minimum t_max).
        // Exactly one axis per step, never two at once: stepping both across a
        // shared corner would skip the two cells that meet there, which is a
        // projectile passing through a corner it could not fit through.
        if self.t_max.x < self.t_max.y {
            if self.t_max.x < self.t_max.z {
                self.t_entry = self.t_max.x;
                self.current_pos.x += self.step.x;
                self.t_max.x += self.t_delta.x;
            } else {
                self.t_entry = self.t_max.z;
                self.current_pos.z += self.step.z;
                self.t_max.z += self.t_delta.z;
            }
        } else if self.t_max.y < self.t_max.z {
            self.t_entry = self.t_max.y;
            self.current_pos.y += self.step.y;
            self.t_max.y += self.t_delta.y;
        } else {
            self.t_entry = self.t_max.z;
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

    #[test]
    fn traversal_starts_in_the_cell_containing_a_negative_origin() {
        // `as_ivec3` truncates towards zero, so this used to start at cell 0.
        let ray = Ray::new(Vec3::new(-0.5, -0.5, -0.5), Vec3::X);
        let first = ray
            .voxel_traversal(IVec3::MIN, IVec3::MAX)
            .next()
            .expect("a traversal yields the cell it starts in");
        assert_eq!(first, IVec3::new(-1, -1, -1));
    }

    #[test]
    fn a_bounded_traversal_stops_at_the_end_of_the_segment() {
        // The bound is closed: a cell entered at exactly `t == max_t` is
        // reported, because the segment does touch it and a collision box
        // flush against that boundary is a real contact. Here the far end is
        // x == 4.0, so cell 4 is the last one, and cell 5 is not.
        let ray = Ray::from_points(Vec3::new(0.5, 0.5, 0.5), Vec3::new(4.0, 0.5, 0.5));
        let voxels = ray
            .voxel_traversal_until(IVec3::MIN, IVec3::MAX, 1.0)
            .collect::<Vec<_>>();
        assert_eq!(voxels, vec![
            IVec3::new(0, 0, 0),
            IVec3::new(1, 0, 0),
            IVec3::new(2, 0, 0),
            IVec3::new(3, 0, 0),
            IVec3::new(4, 0, 0),
        ]);
    }

    #[test]
    fn a_zero_length_ray_yields_only_the_cell_it_is_in() {
        let ray = Ray::new(Vec3::new(0.5, 0.5, 0.5), Vec3::ZERO);
        let voxels = ray
            .voxel_traversal_until(IVec3::MIN, IVec3::MAX, 1.0)
            .collect::<Vec<_>>();
        assert_eq!(voxels, vec![IVec3::ZERO]);
    }

    #[test]
    fn a_diagonal_traversal_steps_one_axis_at_a_time() {
        // Never two axes in one step: a segment that stepped x and y together
        // would jump the corner and skip both cells that meet there.
        let ray = Ray::from_points(Vec3::new(0.5, 0.5, 0.5), Vec3::new(3.5, 3.5, 0.5));
        for pair in ray
            .voxel_traversal_until(IVec3::MIN, IVec3::MAX, 1.0)
            .collect::<Vec<_>>()
            .windows(2)
        {
            let delta = pair[1] - pair[0];
            assert_eq!(
                delta.abs().element_sum(),
                1,
                "stepped {delta} in one move, from {} to {}",
                pair[0],
                pair[1]
            );
        }
    }
}
