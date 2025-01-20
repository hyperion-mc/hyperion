use std::{fmt::{Debug, Display}, ops::Add};
use glam::{Vec3, Vec3A}; 
use ordered_float::NotNan;
use serde::{Deserialize, Serialize};
use crate::ray::Ray;

pub trait HasAabb {
    fn aabb(&self) -> Aabb;
}

#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    
    min: Vec3A,
    max: Vec3A,
    
    #[serde(skip)]
    center: Vec3A,
    #[serde(skip)]
    half_extents: Vec3A,
}

impl Default for Aabb {
    #[inline(always)]
    fn default() -> Self {
        Self::NULL
    }
}

impl HasAabb for Aabb {
    #[inline(always)]
    fn aabb(&self) -> Aabb {
        *self
    }
}

impl Aabb {
    pub const EVERYTHING: Self = Self {
        min: Vec3A::splat(f32::NEG_INFINITY),
        max: Vec3A::splat(f32::INFINITY),
        center: Vec3A::ZERO,
        half_extents: Vec3A::splat(f32::INFINITY),
    };

    pub const NULL: Self = Self {
        min: Vec3A::splat(f32::INFINITY),
        max: Vec3A::splat(f32::NEG_INFINITY),
        center: Vec3A::ZERO,
        half_extents: Vec3A::ZERO,
    };

    #[inline]
    pub fn new(min: impl Into<Vec3>, max: impl Into<Vec3>) -> Self {
        let min = Vec3A::from(min.into());
        let max = Vec3A::from(max.into());
        let center = (min + max) * 0.5;
        let half_extents = (max - min) * 0.5;
        Self { min, max, center, half_extents }
    }

    // Fast SIMD-optimized collision check
    #[inline(always)]
    pub fn collides(&self, other: &Self) -> bool {
        // SIMD comparison
        let min_cmp = self.min.cmple(other.max);
        let max_cmp = self.max.cmpge(other.min);
        (min_cmp & max_cmp).all()
    }

    // Optimized point SIMD collision 
    #[inline(always)]
    pub fn collides_point(&self, point: Vec3) -> bool {
        let point = Vec3A::from(point);
        (point.cmpge(self.min) & point.cmple(self.max)).all()
    }

    // Fast ray intersection using cached values & 
    #[inline]
    pub fn intersect_ray(&self, ray: &Ray) -> Option<NotNan<f32>> {
        let origin = Vec3A::from(ray.origin());
        
        let to_center = self.center - origin;
        let abs_to_center = to_center.abs();
        if abs_to_center.cmpgt(self.half_extents).any() {
            let dir = Vec3A::from(ray.direction());
            if abs_to_center.dot(dir) < 0.0 {
                return None;
            }
        }

        if self.collides_point(ray.origin()) {
            return Some(NotNan::new(0.0).unwrap());
        }

        let inv_dir = Vec3A::from(ray.inv_direction());
        let t1 = (self.min - origin) * inv_dir;
        let t2 = (self.max - origin) * inv_dir;

        let t_min = t1.min(t2);
        let t_max = t1.max(t2);

        let t_enter = t_min.max_element();
        let t_exit = t_max.min_element();

        if t_enter <= t_exit && t_exit >= 0.0 {
            Some(NotNan::new(t_enter.max(0.0)).unwrap())
        } else {
            None
        }
    }

    // Optimized expansion using SIMD
    #[inline]
    pub fn expand(mut self, amount: f32) -> Self {
        let delta = Vec3A::splat(amount);
        self.min -= delta;
        self.max += delta;
        self.half_extents += delta;
        self
    }

    // SIMD-optimized volume calculation
    #[inline(always)]
    pub fn volume(&self) -> f32 {
        let dims = self.max - self.min;
        dims.x * dims.y * dims.z
    }

    // SIMD-optimized surface area calculation
    #[inline(always)]
    pub fn surface_area(&self) -> f32 {
        let dims = self.max - self.min;
        2.0 * (dims.x * dims.y + dims.y * dims.z + dims.z * dims.x)
    }

    // Optimized distance calculation using SIMD
    #[inline]
    pub fn dist2(&self, point: Vec3) -> f64 {
        let point = Vec3A::from(point);
        let clamped = point.clamp(self.min, self.max);
        let diff = point - clamped;
        diff.length_squared() as f64
    }

    // Optimized overlap check returning new AABB
    #[inline]
    pub fn overlap(a: &Self, b: &Self) -> Option<Self> {
        let min = a.min.max(b.min);
        let max = a.max.min(b.max);
        
        if min.cmplt(max).all() {
            let center = (min + max) * 0.5;
            let half_extents = (max - min) * 0.5;
            Some(Self { min, max, center, half_extents })
        } else {
            None
        }
    }

    // Optimized batch processing for multiple AABBs
    #[inline]
    pub fn containing<T: HasAabb>(input: &[T]) -> Self {
        if input.is_empty() {
            return Self::NULL;
        }
    
        let first = input[0].aabb();
        let mut min = first.min;
        let mut max = first.max;
    
        // Process 4 elements at a time
        for chunk in input[1..].chunks_exact(4) {
            let a = &chunk[0];
            let b = &chunk[1];
            let c = &chunk[2];
            let d = &chunk[3];
            let aabbs = [a.aabb(), b.aabb(), c.aabb(), d.aabb()];
        
            min = min
                .min(aabbs[0].min)
                .min(aabbs[1].min)
                .min(aabbs[2].min)
                .min(aabbs[3].min);
            max = max
                .max(aabbs[0].max)
                .max(aabbs[1].max)
                .max(aabbs[2].max)
                .max(aabbs[3].max);
        }
        
    
        let remainder = input[1 + input[1..].len() / 4 * 4..].iter();
        for item in remainder {
            let aabb = item.aabb();
            min = min.min(aabb.min);
            max = max.max(aabb.max);
        }
    
        let center = (min + max) * 0.5;
        let half_extents = (max - min) * 0.5;
        Self {
            min,
            max,
            center,
            half_extents,
        }
    }
} 
// Implement necessary traits
impl Add<Vec3> for Aabb {
    type Output = Self;

    #[inline(always)]
    fn add(self, rhs: Vec3) -> Self::Output {
        let rhs = Vec3A::from(rhs);
        Self {
            min: self.min + rhs,
            max: self.max + rhs,
            center: self.center + rhs,
            half_extents: self.half_extents,
        }
    }
}

impl Debug for Aabb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl Display for Aabb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:.2}, {:.2}, {:.2}] -> [{:.2}, {:.2}, {:.2}]",
            self.min.x, self.min.y, self.min.z,
            self.max.x, self.max.y, self.max.z
        )
    }
}

impl<T: HasAabb> From<&[T]> for Aabb {
    fn from(elements: &[T]) -> Self {
        Self::containing(elements)
    }
}











