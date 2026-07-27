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
        let infinity = Vec3::splat(f32::INFINITY);
        let neg_infinity = Vec3::splat(f32::NEG_INFINITY);
        let (min, max) = iter
            .into_iter()
            .fold((infinity, neg_infinity), |(min, max), aabb| {
                (min.min(aabb.min), max.max(aabb.max))
            });
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

    #[inline]
    #[allow(clippy::missing_const_for_fn, reason = "this is a false positive")]
    #[must_use]
    pub fn move_by(&self, offset: Vec3) -> Self {
        Self {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    #[must_use]
    pub fn overlap(a: &Self, b: &Self) -> Option<Self> {
        let min = a.min.max(b.min);
        let max = a.max.min(b.max);
        if min.cmplt(max).all() {
            Some(Self { min, max })
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub fn collides(&self, other: &Self) -> bool {
        (self.min.cmple(other.max) & self.max.cmpge(other.min)).all()
    }

    #[inline]
    #[must_use]
    pub fn collides_point(&self, point: Vec3) -> bool {
        (self.min.cmple(point) & point.cmple(self.max)).all()
    }

    #[must_use]
    pub fn batch_collides(&self, others: &[Self]) -> Vec<bool> {
        others.iter().map(|other| self.collides(other)).collect()
    }

    #[must_use]
    pub fn dist2(&self, point: Vec3) -> f64 {
        let point_d = point.as_dvec3();
        let min_d = self.min.as_dvec3();
        let max_d = self.max.as_dvec3();
        let clamped = point_d.clamp(min_d, max_d);
        (point_d - clamped).length_squared()
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
        let xy = lens.x * lens.y;
        let yz = lens.y * lens.z;
        let xz = lens.x * lens.z;
        2.0 * (xy + yz + xz)
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        let lens = self.lens();
        lens.x * lens.y * lens.z
    }

    #[inline]
    #[must_use]
    pub fn intersect_ray(&self, ray: &Ray) -> Option<NotNan<f32>> {
        let origin = ray.origin();
        let dir = ray.direction();
        let inv_dir = ray.inv_direction();

        let mut t1 = (self.min - origin) * inv_dir;
        let mut t2 = (self.max - origin) * inv_dir;

        for axis in 0..3 {
            if dir[axis] == 0.0 {
                if !(self.min[axis] <= origin[axis] && origin[axis] <= self.max[axis]) {
                    return None;
                }
                t1[axis] = -f32::INFINITY;
                t2[axis] = f32::INFINITY;
            }
        }

        let t_min = t1.min(t2).max(Vec3::splat(0.0));
        let t_max = t1.max(t2);

        if t_min.max_element() <= t_max.min_element() {
            Some(NotNan::new(t_min.max_element()).unwrap())
        } else {
            None
        }
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
    pub const fn mid_x(&self) -> f32 {
        f32::midpoint(self.min.x, self.max.x)
    }

    #[must_use]
    pub const fn mid_y(&self) -> f32 {
        f32::midpoint(self.min.y, self.max.y)
    }

    #[must_use]
    pub const fn mid_z(&self) -> f32 {
        f32::midpoint(self.min.z, self.max.z)
    }

    #[inline]
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
