//! A Minecraft world is 60 million blocks long.
//! Each chunk is 16 blocks long.
//! This means that the chunk length is 60 million / 16 = 3,750,000 chunks
//! 2^16 = 65,536 and 2^32 is 4,294,967,296.
//!
//! While we should be using more than i16 for chunk coordinates, this is for minigame servers and we are fine
//! using it as we are optimizing for performance
use std::fmt::{Debug, Formatter};

use glam::U16Vec2;
use more_asserts::debug_assert_le;

use crate::Point;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Aabb {
    // 64 bit
    pub min: glam::I16Vec2, // 32 bit
    pub max: glam::I16Vec2, // 32 bit
}

impl Aabb {
    pub const INVALID: Self = Self {
        min: glam::I16Vec2::new(i16::MAX, i16::MAX),
        max: glam::I16Vec2::new(i16::MIN, i16::MIN),
    };
}

impl Debug for Aabb {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {}", self.min, self.max)
    }
}

impl Aabb {
    #[must_use]
    pub const fn new(min: glam::I16Vec2, max: glam::I16Vec2) -> Self {
        Self { min, max }
    }

    #[must_use]
    pub const fn point(elem: glam::I16Vec2) -> Self {
        Self::new(elem, elem)
    }

    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    // todo: test
    #[must_use]
    pub const fn contains_point(self, point: glam::I16Vec2) -> bool {
        self.min.x <= point.x
            && self.max.x >= point.x
            && self.min.y <= point.y
            && self.max.y >= point.y
    }

    #[must_use]
    pub fn to_unit(self) -> Option<glam::I16Vec2> {
        if self.min == self.max {
            Some(self.min)
        } else {
            None
        }
    }

    // todo: test
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    #[must_use]
    pub fn min_max_distance2(self, point: glam::I16Vec2) -> (u32, u32) {
        let this_lens = U16Vec2::from(self.lens());

        let enclosing = self.enclose(point);

        let exterior_lens = U16Vec2::from(enclosing.lens());
        let enclosing_lens = exterior_lens - this_lens;

        let exterior_lens = exterior_lens.as_uvec2();
        let enclosing_lens = enclosing_lens.as_uvec2();

        debug_assert_le!(exterior_lens.x, u32::from(u16::MAX));
        debug_assert_le!(exterior_lens.y, u32::from(u16::MAX));

        let min_dist2 = enclosing_lens.length_squared();
        let max_dist2 = exterior_lens.length_squared();

        (min_dist2, max_dist2)
    }

    pub fn enclosing_aabb<I: Point>(elems: impl IntoIterator<Item = I>) -> Self {
        let elems = elems.into_iter();
        // 16 bits
        let mut min = glam::I16Vec2::MAX;
        let mut max = glam::I16Vec2::MIN;

        for elem in elems {
            let elem = elem.point();
            min = min.min(elem);
            max = max.max(elem);
        }

        Self::new(min, max)
    }

    #[must_use]
    pub fn enclose(self, point: glam::I16Vec2) -> Self {
        let mut min = self.min;
        let mut max = self.max;

        min = min.min(point);
        max = max.max(point);

        Self::new(min, max)
    }

    #[must_use]
    pub const fn lens(self) -> [u16; 2] {
        let lx = self.max.x.abs_diff(self.min.x);
        let ly = self.max.y.abs_diff(self.min.y);
        [lx, ly]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lens() {
        let aabb = Aabb::new(glam::I16Vec2::new(0, 0), glam::I16Vec2::new(10, 10));
        let lens = aabb.lens();
        assert_eq!(lens[0], 10);
        assert_eq!(lens[1], 10);
    }

    #[test]
    fn test_enclosing_aabb() {
        let points = vec![
            glam::I16Vec2::new(1, 2),
            glam::I16Vec2::new(3, 4),
            glam::I16Vec2::new(5, 6),
        ];

        let aabb = Aabb::enclosing_aabb(&points);
        assert_eq!(aabb.min, glam::I16Vec2::new(1, 2));
        assert_eq!(aabb.max, glam::I16Vec2::new(5, 6));
    }

    #[test]
    fn test_lens_negative() {
        let aabb = Aabb::new(glam::I16Vec2::new(-10, -10), glam::I16Vec2::new(10, 10));
        let lens = aabb.lens();
        assert_eq!(lens[0], 20);
        assert_eq!(lens[1], 20);
    }

    #[test]
    fn test_lens_zero() {
        let aabb = Aabb::new(glam::I16Vec2::new(0, 0), glam::I16Vec2::new(0, 0));
        let lens = aabb.lens();
        assert_eq!(lens[0], 0);
        assert_eq!(lens[1], 0);
    }
}
