//! Sweeping a segment through a voxel grid and stopping at the first solid
//! surface it meets.
//!
//! The one implementation of "what does this projectile hit". Everything that
//! moves a projectile a whole tick at a time needs it, and the tempting cheap
//! version -- sample the endpoint, ask whether that block is solid -- is a hole
//! rather than a test: an arrow travelling sixty blocks a second covers three
//! blocks between ticks, so two of every three blocks it passes through are
//! never looked at and a one-block wall is something it flies straight over.
//!
//! [`first_block_hit`] is generic over where the shapes come from, deliberately.
//! The block store answers from loaded chunks and a test answers from a set of
//! coordinates it wrote by hand, and the traversal that decides which cells to
//! ask about is the same code in both cases. That is what makes the unit tests
//! below evidence about the shipped path rather than about a second copy of it.

use glam::{IVec3, Vec3};

use crate::{aabb::Aabb, ray::Ray};

/// Where a swept segment first met a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockHit {
    /// How far along the segment contact happened, as a fraction in `0.0..=1.0`.
    ///
    /// A fraction and not a distance in blocks, because that is what the caller
    /// has to compare against: a hit at `time` is at `from.lerp(to, time)`, and
    /// two candidate hits on the same segment are ordered by it without anyone
    /// having to agree on a unit first.
    pub time: f32,
    /// The block that stopped it.
    pub block: IVec3,
    /// Where on that block's surface contact happened.
    pub point: Vec3,
    /// The unit normal of the face crossed, pointing out of the block.
    ///
    /// [`Vec3::ZERO`] when the segment began already inside the shape, because
    /// then no face was crossed and any answer would be invented.
    pub normal: Vec3,
}

/// The first block surface on the segment `from` -> `to`, or `None` if it is
/// clear.
///
/// `shapes` is asked for the collision boxes of one block, in that block's own
/// coordinates (a full cube is `(0,0,0)..(1,1,1)`); it is called at most once
/// per cell the segment passes through, in the order they are met. An empty
/// iterator means the segment passes through -- air, and anything else with no
/// collision box, are the same answer.
///
/// A segment that starts inside a solid block hits it at `time == 0.0`. That is
/// the honest answer rather than an edge case to skip past: an arrow loosed
/// from inside a wall has not travelled anywhere before it is stopped.
pub fn first_block_hit<I>(
    from: Vec3,
    to: Vec3,
    mut shapes: impl FnMut(IVec3) -> I,
) -> Option<BlockHit>
where
    I: IntoIterator<Item = Aabb>,
{
    let ray = Ray::from_points(from, to);

    // The segment is the bound, not a box drawn around the world. Traversal
    // stops at `t == 1` -- one tick of travel -- so a shot into an empty sky
    // walks the handful of cells it actually crosses rather than every cell out
    // to the edge of the coordinate space.
    for block in ray.voxel_traversal_until(IVec3::MIN, IVec3::MAX, 1.0) {
        let offset = block.as_vec3();
        let mut nearest: Option<(f32, Aabb)> = None;

        for shape in shapes(block) {
            let shape = shape + offset;
            let Some(time) = shape.intersect_ray(&ray) else {
                continue;
            };
            let time = time.into_inner();
            // Past the end of this tick's travel: a real hit, on a later tick.
            if time > 1.0 {
                continue;
            }
            if nearest.is_none_or(|(nearest, _)| time < nearest) {
                nearest = Some((time, shape));
            }
        }

        // Cells arrive in increasing order of entry time and a block's collision
        // boxes are inside it, so the first cell with any hit holds the earliest
        // hit on the whole segment. No need to look further.
        if let Some((time, shape)) = nearest {
            return Some(BlockHit {
                time,
                block,
                point: ray.at(time),
                normal: face_normal(shape, ray, time),
            });
        }
    }

    None
}

/// Which face of `shape` the ray crossed to enter it at `time`.
///
/// The slab entry times recomputed rather than carried out of
/// [`Aabb::intersect_ray`]: it returns the largest of the three and the axis it
/// came from is the answer here, so the choice is between widening that
/// signature for one caller and redoing three divisions. The divisions are
/// cheaper than the coupling.
fn face_normal(shape: Aabb, ray: Ray, time: f32) -> Vec3 {
    let direction = ray.direction();
    let origin = ray.origin();

    let mut normal = Vec3::ZERO;
    // The unclamped slab entry: the largest of the three per-axis entry times,
    // which is the intersection time before `intersect_ray` clamps it to zero.
    // Taking the maximum rather than filtering against `time` is what keeps
    // this robust -- the winning axis reproduces `time` to the last bit only
    // when it is computed the same way, and a comparison against a separately
    // rounded value picks no axis at all every so often.
    let mut entered_at = f32::NEG_INFINITY;

    for axis in 0..3 {
        // Parallel to this pair of faces: it crossed neither.
        if direction[axis] == 0.0 {
            continue;
        }
        let sign = if direction[axis] > 0.0 { 1.0 } else { -1.0 };
        let near = if sign > 0.0 {
            shape.min[axis]
        } else {
            shape.max[axis]
        };
        let entry = (near - origin[axis]) / direction[axis];
        if entry > entered_at {
            entered_at = entry;
            normal = Vec3::ZERO;
            normal[axis] = -sign;
        }
    }

    // Entered before the segment began, which means it did not enter at all:
    // it started inside. No face was crossed, so there is no face normal.
    debug_assert!(
        entered_at <= time || time > 0.0,
        "a hit at t == 0 cannot have entered after it"
    );
    if entered_at < 0.0 { Vec3::ZERO } else { normal }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// A world of full cubes at the listed coordinates, which is what a test
    /// about traversal wants: the question is which cells get looked at and in
    /// what order, and a slab would only make the arithmetic harder to read.
    fn cubes(solid: impl IntoIterator<Item = IVec3>) -> impl Fn(IVec3) -> Option<Aabb> {
        let solid: HashSet<IVec3> = solid.into_iter().collect();
        move |block| {
            solid
                .contains(&block)
                .then(|| Aabb::new(Vec3::ZERO, Vec3::ONE))
        }
    }

    #[test]
    fn axis_aligned_shot_stops_at_the_near_face() {
        let world = cubes([IVec3::new(5, 0, 0)]);
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(10.5, 0.5, 0.5), &world)
            .expect("a wall five blocks along +X is on the segment");

        assert_eq!(hit.block, IVec3::new(5, 0, 0));
        // Entered at x == 5, having started at x == 0.5 and aiming for 10.5.
        assert!((hit.point.x - 5.0).abs() < 1e-4, "point: {}", hit.point);
        assert!((hit.time - 0.45).abs() < 1e-4, "time: {}", hit.time);
        assert_eq!(hit.normal, Vec3::NEG_X);
    }

    #[test]
    fn open_ground_is_a_miss() {
        let world = cubes([IVec3::new(5, 0, 0)]);
        // Same wall, one block higher than the segment.
        let hit = first_block_hit(Vec3::new(0.5, 1.5, 0.5), Vec3::new(10.5, 1.5, 0.5), &world);
        assert_eq!(hit, None);
    }

    #[test]
    fn a_wall_beyond_the_end_of_the_segment_is_not_hit_yet() {
        let world = cubes([IVec3::new(5, 0, 0)]);
        // One tick's travel that stops short of the wall. The old unbounded
        // scan reported this as a hit and an arrow stopped in mid-air metres
        // before anything was in the way.
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(3.5, 0.5, 0.5), &world);
        assert_eq!(hit, None);
    }

    #[test]
    fn a_fast_shot_cannot_tunnel_through_a_one_block_wall() {
        let world = cubes([IVec3::new(5, 0, 0)]);
        // Sixty blocks in one step: both endpoints are in air and only the
        // cells between them say otherwise.
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(60.5, 0.5, 0.5), &world)
            .expect("the wall is between the endpoints even though neither is in it");
        assert_eq!(hit.block, IVec3::new(5, 0, 0));
    }

    #[test]
    fn a_diagonal_shot_meets_the_blocks_it_passes_through() {
        // A staircase of blocks along the diagonal. The segment crosses the
        // second one; the first sits behind the start.
        let world = cubes([IVec3::new(3, 3, 0)]);
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(6.5, 6.5, 0.5), &world)
            .expect("the diagonal passes through (3, 3, 0)");
        assert_eq!(hit.block, IVec3::new(3, 3, 0));
        // Entered through whichever face it reached first; on an exact diagonal
        // through a corner that is a tie, and either face is a truthful answer.
        assert!(
            hit.normal == Vec3::NEG_X || hit.normal == Vec3::NEG_Y,
            "normal: {}",
            hit.normal
        );
    }

    #[test]
    fn a_corner_the_segment_misses_does_not_stop_it() {
        // Two blocks meeting at a corner with a gap on the diagonal between
        // them. A traversal that steps both axes at once would step through the
        // shared corner and report neither; one that steps a single axis per
        // cell visits one of the two and stops.
        let world = cubes([IVec3::new(1, 0, 0), IVec3::new(0, 1, 0)]);
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.5, 1.5, 0.5), &world)
            .expect("a segment through the shared corner is stopped by one of the two");
        assert!(
            hit.block == IVec3::new(1, 0, 0) || hit.block == IVec3::new(0, 1, 0),
            "block: {}",
            hit.block
        );
    }

    #[test]
    fn negative_coordinates_traverse_the_same_as_positive_ones() {
        // `as_ivec3` truncates towards zero, so a start at x == -0.5 used to be
        // read as cell 0 rather than cell -1: everything fired in the negative
        // half of a map began its traversal one cell off, and half of every map
        // is in the negative half.
        let world = cubes([IVec3::new(-5, -1, -1)]);
        let hit = first_block_hit(
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(-10.5, -0.5, -0.5),
            &world,
        )
        .expect("a wall five blocks along -X is on the segment");

        assert_eq!(hit.block, IVec3::new(-5, -1, -1));
        // Entered through the +X face, at x == -4.
        assert!((hit.point.x + 4.0).abs() < 1e-4, "point: {}", hit.point);
        assert_eq!(hit.normal, Vec3::X);
    }

    #[test]
    fn every_axis_and_sign_stops_at_the_same_distance() {
        for axis in 0..3 {
            for sign in [1.0_f32, -1.0] {
                let mut direction = Vec3::ZERO;
                direction[axis] = sign;

                let mut wall = IVec3::ZERO;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "5 and -6 are exactly representable"
                )]
                let along = (5.0 * sign) as i32 - i32::from(sign < 0.0);
                wall[axis] = along;

                let world = cubes([wall]);
                let from = Vec3::splat(0.5);
                let hit = first_block_hit(from, from + direction * 10.0, &world)
                    .unwrap_or_else(|| panic!("axis {axis} sign {sign} should hit {wall}"));

                assert_eq!(hit.block, wall, "axis {axis} sign {sign}");
                let mut expected = Vec3::ZERO;
                expected[axis] = -sign;
                assert_eq!(hit.normal, expected, "axis {axis} sign {sign}");
            }
        }
    }

    #[test]
    fn a_segment_starting_inside_a_block_is_stopped_where_it_started() {
        let world = cubes([IVec3::ZERO]);
        let hit = first_block_hit(Vec3::splat(0.5), Vec3::new(10.5, 0.5, 0.5), &world)
            .expect("a shot from inside a wall does not get out of it");

        assert_eq!(hit.block, IVec3::ZERO);
        assert!((hit.time - 0.0).abs() < 1e-6, "time: {}", hit.time);
        assert_eq!(hit.point, Vec3::splat(0.5));
        // No face was crossed, so there is no face normal to report.
        assert_eq!(hit.normal, Vec3::ZERO);
    }

    #[test]
    fn a_zero_length_segment_in_air_hits_nothing() {
        let world = cubes([IVec3::new(5, 0, 0)]);
        let at = Vec3::splat(0.5);
        assert_eq!(first_block_hit(at, at, &world), None);
    }

    #[test]
    fn the_nearest_of_several_blocks_is_the_one_reported() {
        let world = cubes([
            IVec3::new(2, 0, 0),
            IVec3::new(5, 0, 0),
            IVec3::new(9, 0, 0),
        ]);
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(10.5, 0.5, 0.5), &world)
            .expect("three walls ahead, one of them first");
        assert_eq!(hit.block, IVec3::new(2, 0, 0));
    }

    #[test]
    fn a_partial_shape_is_missed_where_a_full_cube_would_be_hit() {
        // The bottom half of the cell only: a slab. A segment through the top
        // half passes over it, which is the whole reason shapes are asked for
        // per block rather than a solid/not-solid bit.
        let slab = |block: IVec3| {
            (block == IVec3::new(5, 0, 0)).then(|| Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0)))
        };
        assert_eq!(
            first_block_hit(Vec3::new(0.5, 0.8, 0.5), Vec3::new(10.5, 0.8, 0.5), &slab),
            None
        );
        let hit = first_block_hit(Vec3::new(0.5, 0.2, 0.5), Vec3::new(10.5, 0.2, 0.5), &slab)
            .expect("a segment through the lower half meets the slab");
        assert_eq!(hit.block, IVec3::new(5, 0, 0));
    }
}
