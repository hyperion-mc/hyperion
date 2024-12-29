use std::{
    fmt::{Debug, Display},
    ops::Add,
};

use glam::Vec3;
use ordered_float::NotNan;
use serde::{Deserialize, Serialize};

use crate::ray::Ray;

pub trait HasAabb {
    fn aabb(&self) -> Aabb;
}

impl HasAabb for Aabb {
    fn aabb(&self) -> Aabb {
        *self
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Ord, PartialOrd, Hash)]
pub struct OrderedAabb {
    min_x: NotNan<f32>,
    min_y: NotNan<f32>,
    min_z: NotNan<f32>,
    max_x: NotNan<f32>,
    max_y: NotNan<f32>,
    max_z: NotNan<f32>,
}

impl TryFrom<Aabb> for OrderedAabb {
    type Error = ordered_float::FloatIsNan;

    fn try_from(value: Aabb) -> Result<Self, Self::Error> {
        Ok(Self {
            min_x: value.min.x.try_into()?,
            min_y: value.min.y.try_into()?,
            min_z: value.min.z.try_into()?,
            max_x: value.max.x.try_into()?,
            max_y: value.max.y.try_into()?,
            max_z: value.max.z.try_into()?,
        })
    }
}

#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl From<(f32, f32, f32, f32, f32, f32)> for Aabb {
    fn from(value: (f32, f32, f32, f32, f32, f32)) -> Self {
        let value: [f32; 6] = value.into();
        Self::from(value)
    }
}

impl From<[f32; 6]> for Aabb {
    fn from(value: [f32; 6]) -> Self {
        let [min_x, min_y, min_z, max_x, max_y, max_z] = value;
        let min = Vec3::new(min_x, min_y, min_z);
        let max = Vec3::new(max_x, max_y, max_z);

        Self { min, max }
    }
}

impl FromIterator<Self> for Aabb {
    fn from_iter<T: IntoIterator<Item = Self>>(iter: T) -> Self {
        let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);

        for aabb in iter {
            min = min.min(aabb.min);
            max = max.max(aabb.max);
        }

        Self { min, max }
    }
}

impl Debug for Aabb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl Display for Aabb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // write [0.00, 0.00, 0.00] -> [1.00, 1.00, 1.00]
        write!(
            f,
            "[{:.2}, {:.2}, {:.2}] -> [{:.2}, {:.2}, {:.2}]",
            self.min.x, self.min.y, self.min.z, self.max.x, self.max.y, self.max.z
        )
    }
}

impl Add<Vec3> for Aabb {
    type Output = Self;

    fn add(self, rhs: Vec3) -> Self::Output {
        Self {
            min: self.min + rhs,
            max: self.max + rhs,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Ord, PartialOrd, Hash)]
pub struct CheckableAabb {
    pub min: [NotNan<f32>; 3],
    pub max: [NotNan<f32>; 3],
}

impl TryFrom<Aabb> for CheckableAabb {
    type Error = ordered_float::FloatIsNan;

    fn try_from(value: Aabb) -> Result<Self, Self::Error> {
        Ok(Self {
            min: [
                NotNan::new(value.min.x)?,
                NotNan::new(value.min.y)?,
                NotNan::new(value.min.z)?,
            ],
            max: [
                NotNan::new(value.max.x)?,
                NotNan::new(value.max.y)?,
                NotNan::new(value.max.z)?,
            ],
        })
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::NULL
    }
}

impl Aabb {
    pub const EVERYTHING: Self = Self {
        min: Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
        max: Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
    };
    pub const NULL: Self = Self {
        min: Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
        max: Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
    };

    #[must_use]
    pub fn new(min: impl Into<Vec3>, max: impl Into<Vec3>) -> Self {
        let min = min.into();
        let max = max.into();
        Self { min, max }
    }

    #[must_use]
    pub fn shrink(self, amount: f32) -> Self {
        Self::expand(self, -amount)
    }

    #[must_use]
    pub fn move_to_feet(&self, feet: Vec3) -> Self {
        let half_width = (self.max.x - self.min.x) / 2.0;
        let height = self.max.y - self.min.y;

        let min = Vec3::new(feet.x - half_width, feet.y, feet.z - half_width);
        let max = Vec3::new(feet.x + half_width, feet.y + height, feet.z + half_width);

        Self { min, max }
    }

    #[must_use]
    pub fn create(feet: Vec3, width: f32, height: f32) -> Self {
        let half_width = width / 2.0;

        let min = Vec3::new(feet.x - half_width, feet.y, feet.z - half_width);
        let max = Vec3::new(feet.x + half_width, feet.y + height, feet.z + half_width);

        Self { min, max }
    }

    #[must_use]
    pub fn move_by(&self, offset: Vec3) -> Self {
        Self {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    #[must_use]
    pub fn overlap(a: &Self, b: &Self) -> Option<Self> {
        let min_x = a.min.x.max(b.min.x);
        let min_y = a.min.y.max(b.min.y);
        let min_z = a.min.z.max(b.min.z);

        let max_x = a.max.x.min(b.max.x);
        let max_y = a.max.y.min(b.max.y);
        let max_z = a.max.z.min(b.max.z);

        // Check if there is an overlap. If any dimension does not overlap, return None.
        if min_x < max_x && min_y < max_y && min_z < max_z {
            Some(Self {
                min: Vec3::new(min_x, min_y, min_z),
                max: Vec3::new(max_x, max_y, max_z),
            })
        } else {
            None
        }
    }

    #[must_use]
    pub fn collides(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    #[must_use]
    pub fn collides_point(&self, point: Vec3) -> bool {
        self.min.x <= point.x
            && point.x <= self.max.x
            && self.min.y <= point.y
            && point.y <= self.max.y
            && self.min.z <= point.z
            && point.z <= self.max.z
    }

    #[must_use]
    pub fn dist2(&self, point: Vec3) -> f64 {
        let point = point.as_dvec3();
        // Clamp the point into the box volume.
        let clamped = point.clamp(self.min.as_dvec3(), self.max.as_dvec3());

        // Distance vector from point to the clamped point inside the box.
        let diff = point - clamped;

        // The squared distance.
        diff.length_squared()
    }

    pub fn overlaps<'a, T>(
        &'a self,
        elements: impl Iterator<Item = &'a T>,
    ) -> impl Iterator<Item = &'a T>
    where
        T: HasAabb + 'a,
    {
        elements.filter(|element| self.collides(&element.aabb()))
    }

    #[must_use]
    pub fn surface_area(&self) -> f32 {
        let lens = self.lens();
        2.0 * lens
            .z
            .mul_add(lens.x, lens.x.mul_add(lens.y, lens.y * lens.z))
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        let lens = self.lens();
        lens.x * lens.y * lens.z
    }

    #[must_use]
    pub fn intersect_ray(&self, ray: &Ray) -> Option<NotNan<f32>> {
        let origin = ray.origin();

        // If the ray is originating inside the AABB, we can immediately return.
        if self.contains_point(origin) {
            return Some(NotNan::new(0.0).unwrap());
        }

        let dir = ray.direction();
        let inv_dir = ray.inv_direction();

        // Initialize t_min and t_max to the range of possible values
        let (mut t_min, mut t_max) = (f32::NEG_INFINITY, f32::INFINITY);

        // X-axis
        if dir.x != 0.0 {
            let tx1 = (self.min.x - origin.x) * inv_dir.x;
            let tx2 = (self.max.x - origin.x) * inv_dir.x;
            t_min = t_min.max(tx1.min(tx2));
            t_max = t_max.min(tx1.max(tx2));
        } else if origin.x < self.min.x || origin.x > self.max.x {
            return None; // Ray is parallel to X slab and outside the slab
        }

        // Y-axis
        if dir.y != 0.0 {
            let ty1 = (self.min.y - origin.y) * inv_dir.y;
            let ty2 = (self.max.y - origin.y) * inv_dir.y;
            t_min = t_min.max(ty1.min(ty2));
            t_max = t_max.min(ty1.max(ty2));
        } else if origin.y < self.min.y || origin.y > self.max.y {
            return None; // Ray is parallel to Y slab and outside the slab
        }

        // Z-axis
        if dir.z != 0.0 {
            let tz1 = (self.min.z - origin.z) * inv_dir.z;
            let tz2 = (self.max.z - origin.z) * inv_dir.z;
            t_min = t_min.max(tz1.min(tz2));
            t_max = t_max.min(tz1.max(tz2));
        } else if origin.z < self.min.z || origin.z > self.max.z {
            return None; // Ray is parallel to Z slab and outside the slab
        }

        // At this point, t_min and t_max define the intersection range.
        // If t_min < 0.0, it means we start “behind” the origin; if t_max < 0.0, no intersection in front.
        let t_hit = if t_min >= 0.0 { t_min } else { t_max };
        if t_hit < 0.0 {
            return None;
        }

        Some(NotNan::new(t_hit).unwrap())
    }

    #[must_use]
    pub fn expand(mut self, amount: f32) -> Self {
        self.min -= Vec3::splat(amount);
        self.max += Vec3::splat(amount);
        self
    }

    /// Check if a point is inside the AABB
    #[must_use]
    pub fn contains_point(&self, point: Vec3) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    pub fn expand_to_fit(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    #[must_use]
    pub fn mid(&self) -> Vec3 {
        (self.min + self.max) / 2.0
    }

    #[must_use]
    pub fn mid_x(&self) -> f32 {
        (self.min.x + self.max.x) / 2.0
    }

    #[must_use]
    pub fn mid_y(&self) -> f32 {
        (self.min.y + self.max.y) / 2.0
    }

    #[must_use]
    pub fn mid_z(&self) -> f32 {
        (self.min.z + self.max.z) / 2.0
    }

    #[must_use]
    pub fn lens(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn containing<T: HasAabb>(input: &[T]) -> Self {
        let mut current_min = Vec3::splat(f32::INFINITY);
        let mut current_max = Vec3::splat(f32::NEG_INFINITY);

        for elem in input {
            let elem = elem.aabb();
            current_min = current_min.min(elem.min);
            current_max = current_max.max(elem.max);
        }

        Self {
            min: current_min,
            max: current_max,
        }
    }
}

impl<T: HasAabb> From<&[T]> for Aabb {
    fn from(elements: &[T]) -> Self {
        Self::containing(elements)
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use glam::Vec3;
    use ordered_float::NotNan;

    use crate::{aabb::Aabb, ray::Ray};

    #[test]
    fn test_expand_to_fit() {
        let mut aabb = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        };

        let other = Aabb {
            min: Vec3::new(-1.0, -1.0, -1.0),
            max: Vec3::new(2.0, 2.0, 2.0),
        };

        aabb.expand_to_fit(&other);

        assert_eq!(aabb.min, Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(aabb.max, Vec3::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn containing_returns_correct_aabb_for_multiple_aabbs() {
        let aabbs = vec![
            Aabb {
                min: Vec3::new(0.0, 0.0, 0.0),
                max: Vec3::new(1.0, 1.0, 1.0),
            },
            Aabb {
                min: Vec3::new(-1.0, -1.0, -1.0),
                max: Vec3::new(2.0, 2.0, 2.0),
            },
            Aabb {
                min: Vec3::new(0.5, 0.5, 0.5),
                max: Vec3::new(1.5, 1.5, 1.5),
            },
        ];

        let containing_aabb = Aabb::containing(&aabbs);

        assert_eq!(containing_aabb.min, Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(containing_aabb.max, Vec3::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn containing_returns_correct_aabb_for_single_aabb() {
        let aabbs = vec![Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(1.0, 1.0, 1.0),
        }];

        let containing_aabb = Aabb::containing(&aabbs);

        assert_eq!(containing_aabb.min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(containing_aabb.max, Vec3::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn containing_returns_null_aabb_for_empty_input() {
        let aabbs: Vec<Aabb> = vec![];

        let containing_aabb = Aabb::containing(&aabbs);

        assert_eq!(
            containing_aabb.min,
            Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY)
        );
        assert_eq!(
            containing_aabb.max,
            Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY)
        );
    }

    #[test]
    fn test_ray_aabb_intersection() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // Ray starting outside and hitting the box
        let ray1 = Ray::new(Vec3::new(-2.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(aabb.intersect_ray(&ray1).is_some());

        // Ray starting inside the box
        let ray2 = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(aabb.intersect_ray(&ray2).is_some());

        // Ray missing the box
        let ray3 = Ray::new(Vec3::new(-2.0, 2.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(aabb.intersect_ray(&ray3).is_none());
    }

    #[test]
    fn test_point_containment() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // Test point inside
        assert!(aabb.contains_point(Vec3::new(0.0, 0.0, 0.0)));

        // Test point on boundary
        assert!(aabb.contains_point(Vec3::new(1.0, 0.0, 0.0)));

        // Test point outside
        assert!(!aabb.contains_point(Vec3::new(2.0, 0.0, 0.0)));
    }

    #[test]
    fn test_ray_at() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));

        let point = ray.at(2.0);
        assert_eq!(point, Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn test_degenerate_aabb_as_point() {
        let aabb = Aabb::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(0.0, 1.0, 1.0), Vec3::new(1.0, 0.0, 0.0));
        let intersection = aabb.intersect_ray(&ray);
        assert!(
            intersection.is_some(),
            "Ray should hit the degenerate AABB point"
        );
        assert_relative_eq!(intersection.unwrap().into_inner(), 1.0, max_relative = 1e-6);
    }

    #[test]
    fn test_degenerate_aabb_as_line() {
        let aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
        let ray = Ray::new(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        let intersection = aabb.intersect_ray(&ray);
        assert!(
            intersection.is_some(),
            "Ray should hit the line segment AABB"
        );
        assert_relative_eq!(intersection.unwrap().into_inner(), 1.0, max_relative = 1e-6);
    }

    #[test]
    fn test_ray_touching_aabb_boundary() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        // Ray parallel to one axis and just touches at x = -1
        let ray = Ray::new(Vec3::new(-2.0, 1.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        let intersection = aabb.intersect_ray(&ray);
        assert!(
            intersection.is_some(),
            "Ray should intersect exactly at the boundary x = -1"
        );
        assert_relative_eq!(intersection.unwrap().into_inner(), 1.0, max_relative = 1e-6);
    }

    #[test]
    fn test_ray_near_corner() {
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(0.0, 0.0, 0.0));
        // A ray that "just misses" the corner at (-1,-1,-1)
        let ray = Ray::new(
            Vec3::new(-2.0, -1.000_001, -1.000_001),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let intersection = aabb.intersect_ray(&ray);
        // Depending on precision, this might fail if the intersection logic isn't robust.
        // Checking that we correctly return None or an intersection close to the corner.
        assert!(intersection.is_none(), "Ray should miss by a tiny margin");
    }

    #[test]
    fn test_ray_origin_inside_single_aabb() {
        let aabb = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 10.0));
        let ray = Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(1.0, 0.0, 0.0)); // Inside the box
        let dist = aabb.intersect_ray(&ray);
        assert!(
            dist.is_some(),
            "Ray from inside should intersect at t=0 or near 0"
        );
        assert_relative_eq!(dist.unwrap().into_inner(), 0.0, max_relative = 1e-6);
    }

    #[test]
    fn test_ray_stationary_inside_aabb() {
        let aabb = Aabb::new((0.0, 0.0, 0.0), (10.0, 10.0, 10.0));
        let ray = Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(0.0, 0.0, 0.0));
        // With zero direction, we might choose to say intersection is at t=0 if inside, None if outside.
        let intersection = aabb.intersect_ray(&ray);
        assert_eq!(
            intersection,
            Some(NotNan::new(0.0).unwrap()),
            "Inside and no direction should mean immediate intersection at t=0"
        );
    }

    #[test]
    fn test_ray_just_inside_boundary() {
        let aabb = Aabb::new((0.0, 0.0, 0.0), (1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(0.999_999, 0.5, 0.5), Vec3::new(1.0, 0.0, 0.0));
        let intersection = aabb.intersect_ray(&ray);
        // If inside, intersection should be at t=0.0 or very close.
        assert!(intersection.is_some());
        assert_relative_eq!(intersection.unwrap().into_inner(), 0.0, max_relative = 1e-6);
    }
}
