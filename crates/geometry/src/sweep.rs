//! Sweeping a segment through a voxel grid and stopping at the first solid
//! surface it meets.
//!
//! A transcription of vanilla's own clip, not an implementation of the same
//! idea. `AbstractArrow.tick` resolves its movement through
//! `level().clipIncludingBorder(new ClipContext(from, to, Block.COLLIDER,
//! Fluid.NONE, this))`, and that call is three pieces of Mojang code:
//! `BlockGetter.traverseBlocks` walks the cells, `VoxelShape.clip` asks one
//! block, and `AABB.clip` finds the nearest face of that block's boxes. Each is
//! reproduced below with the divergence it closes named beside it, because
//! "the same algorithm" and "the same answers" are different claims and only
//! the second one is worth anything to a player watching an arrow.
//!
//! The decompiled originals are the ones in
//! `nix build .#minecraft-physics-sources`, at `BlockGetter.java:112`,
//! `VoxelShape.java:147` and `AABB.java:302`. A jar bump that moves them fails
//! that derivation's landmark checks rather than leaving these citations
//! pointing at nothing.
//!
//! # Why the arithmetic is `f64`
//!
//! Vanilla computes in `double` and leans on epsilons of `1.0E-7`: the
//! traversal pushes both endpoints outward by that much, and `AABB.clip`
//! allows that much slack when testing whether a hit lies within a face. At
//! `f32`, one ulp at a coordinate of 65 is `7.6e-6` -- seventy-six times the
//! epsilon -- so every one of those adjustments rounds away to nothing and the
//! transcription would be a transcription in name only. So the endpoints come
//! in as `f32`, widen once, and the whole clip runs in `f64`.
//!
//! That does not make this bit-identical to vanilla, and the remaining gap is
//! named rather than papered over: hyperion holds a projectile's position as
//! `Vec3`, so the *inputs* are already quantised to `f32` where vanilla's are
//! not. What the widening buys is that the boundary cases -- a segment running
//! exactly along `y == 65.0`, an endpoint exactly on a block face -- are
//! decided the way vanilla decides them, and those are exactly the cases where
//! the epsilons are load bearing and where a coordinate is exactly
//! representable in both.
//!
//! [`first_block_hit`] is generic over where the shapes come from,
//! deliberately. The block store answers from loaded chunks and a test answers
//! from a set of coordinates it wrote by hand, and the traversal that decides
//! which cells to ask about is the same code in both cases. That is what makes
//! the unit tests below evidence about the shipped path rather than about a
//! second copy of it.

use glam::{DVec3, IVec3, Vec3};

use crate::aabb::Aabb;

/// How far outside the segment vanilla pushes each endpoint before walking it.
///
/// `BlockGetter.traverseBlocks` lerps both ends by `-1.0E-7`, which moves each
/// one away from the other. It is what stops a segment that ends exactly on a
/// block face from being a coin flip between the two cells that face divides.
const ENDPOINT_EPSILON: f64 = 1.0E-7;

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
    /// For a segment that began inside the shape this is
    /// `Direction.getApproximateNearest(diff).getOpposite()` -- the face it
    /// would have come in through, had it come in -- which is what vanilla
    /// reports rather than nothing. [`Self::inside`] is how a caller tells the
    /// two apart.
    pub normal: Vec3,
    /// Vanilla's `BlockHitResult.isInside`: the segment started within the
    /// block's collision shape rather than crossing into it.
    pub inside: bool,
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
/// A transcription of `BlockGetter.traverseBlocks` (`BlockGetter.java:112`).
/// The cell walk uses endpoints nudged outward by [`ENDPOINT_EPSILON`]; the
/// per-block clip is handed the *original* endpoints, because vanilla's
/// consumer closes over `context.getFrom()` rather than over the lerped locals.
pub fn first_block_hit<I>(
    from: Vec3,
    to: Vec3,
    mut shapes: impl FnMut(IVec3) -> I,
) -> Option<BlockHit>
where
    I: IntoIterator<Item = Aabb>,
{
    // Divergence 3: `if (from.equals(to)) return missFactory.apply(context)`.
    // A zero-length segment is a miss before anything else runs, so it never
    // reaches the start-cell probe below and cannot report the block it is
    // standing in.
    if from == to {
        return None;
    }

    let origin = from.as_dvec3();
    let target = to.as_dvec3();

    // Divergence 1: both endpoints pushed outward, `Mth.lerp(-1.0E-7, a, b)`
    // being `a - 1e-7 * (b - a)`. Only the walk sees these; the clip below
    // gets the originals.
    let walk_end = lerp(target, origin, -ENDPOINT_EPSILON);
    let walk_start = lerp(origin, target, -ENDPOINT_EPSILON);

    let mut block = floor_ivec3(walk_start);
    if let Some(hit) = clip_block(&mut shapes, block, origin, target) {
        return Some(hit);
    }

    let delta = walk_end - walk_start;
    let sign = IVec3::new(sign(delta.x), sign(delta.y), sign(delta.z));
    // `Double.MAX_VALUE` for an axis that does not move, so its `t` never wins
    // a comparison and never advances.
    let step = DVec3::new(
        axis_step(sign.x, delta.x),
        axis_step(sign.y, delta.y),
        axis_step(sign.z, delta.z),
    );
    let mut t = DVec3::new(
        step.x * axis_offset(sign.x, walk_start.x),
        step.y * axis_offset(sign.y, walk_start.y),
        step.z * axis_offset(sign.z, walk_start.z),
    );

    // Divergence 2: the bound is `||`, not `&&` and not a bound on the cell's
    // entry time. An axis that has not yet reached the end of the segment keeps
    // the walk alive, so the last cell visited can be one entered past `t == 1`
    // -- and vanilla clips inside it against the real segment, so a hit there
    // is still a hit at `t <= 1`. Bounding on entry time instead dropped that
    // cell entirely.
    while t.x <= 1.0 || t.y <= 1.0 || t.z <= 1.0 {
        if t.x < t.y {
            if t.x < t.z {
                block.x += sign.x;
                t.x += step.x;
            } else {
                block.z += sign.z;
                t.z += step.z;
            }
        } else if t.y < t.z {
            block.y += sign.y;
            t.y += step.y;
        } else {
            block.z += sign.z;
            t.z += step.z;
        }

        if let Some(hit) = clip_block(&mut shapes, block, origin, target) {
            return Some(hit);
        }
    }

    None
}

/// `Mth.lerp(t, a, b)`.
fn lerp(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    a + (b - a) * t
}

/// `Mth.sign`.
fn sign(value: f64) -> i32 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// `tDelta`: how much `t` advances per cell on this axis.
fn axis_step(sign: i32, delta: f64) -> f64 {
    if sign == 0 {
        f64::MAX
    } else {
        f64::from(sign) / delta
    }
}

/// How far into the first cell the start sits, as a fraction of that cell.
fn axis_offset(sign: i32, start: f64) -> f64 {
    let frac = start - start.floor();
    if sign > 0 { 1.0 - frac } else { frac }
}

const fn floor_ivec3(point: DVec3) -> IVec3 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a block coordinate outside i32 is outside any world; vanilla's Mth.floor casts \
                  the same way"
    )]
    IVec3::new(
        point.x.floor() as i32,
        point.y.floor() as i32,
        point.z.floor() as i32,
    )
}

/// One block's answer: `VoxelShape.clip` (`VoxelShape.java:147`).
///
/// `from` and `to` are the segment the caller asked about, not the walk's
/// nudged copy.
fn clip_block<I>(
    shapes: &mut impl FnMut(IVec3) -> I,
    block: IVec3,
    from: DVec3,
    to: DVec3,
) -> Option<BlockHit>
where
    I: IntoIterator<Item = Aabb>,
{
    let boxes: Vec<Aabb> = shapes(block).into_iter().collect();
    // `if (this.isEmpty()) return null`.
    if boxes.is_empty() {
        return None;
    }

    let diff = to - from;
    // Divergence 5: a segment too short to clip against is not clipped, even
    // though it is long enough for the traversal above to have walked it. The
    // bound is on the squared length, so this is a segment under about 3.2e-4
    // blocks.
    if diff.length_squared() < 1.0E-7 {
        return None;
    }

    // Divergence 4: the start-inside probe. Vanilla asks whether the shape is
    // solid a thousandth of the way along, not at `from` itself, and reports
    // the hit *at that probe point* with the face the segment would have
    // entered by. Reporting it at `from` with no normal -- which is what
    // clamping a slab test to zero produces -- is a different point and a
    // different face.
    let probe = from + diff * 0.001;
    let local = probe - block.as_dvec3();
    if boxes.iter().any(|shape| contains(*shape, local)) {
        return Some(BlockHit {
            time: 0.001,
            block,
            point: probe.as_vec3(),
            normal: -approximate_nearest(diff),
            inside: true,
        });
    }

    clip_boxes(&boxes, block, from, diff)
}

/// Is `point` -- in the block's own coordinates -- within this box?
///
/// Half open, matching `VoxelShape.isFullWide` reached through `findIndex`: the
/// index of a coordinate sitting exactly on a face is the cell *above* it, so a
/// point on a shape's upper surface is outside the shape rather than in it.
fn contains(shape: Aabb, point: DVec3) -> bool {
    let min = shape.min.as_dvec3();
    let max = shape.max.as_dvec3();
    (0..3).all(|axis| min[axis] <= point[axis] && point[axis] < max[axis])
}

/// `Direction.getApproximateNearest`: the axis-aligned direction most nearly
/// along `direction`.
///
/// Vanilla maximises the dot product over `Direction.values()` in the order
/// `DOWN, UP, NORTH, SOUTH, WEST, EAST` with a strict `>`, so a tie goes to the
/// earliest of them -- and a zero vector, which beats nothing, comes out as
/// `NORTH`. Both are reproduced here because a tie is what an exactly diagonal
/// shot produces, which is not a rare input.
fn approximate_nearest(direction: DVec3) -> Vec3 {
    const CANDIDATES: [Vec3; 6] = [
        Vec3::NEG_Y,
        Vec3::Y,
        Vec3::NEG_Z,
        Vec3::Z,
        Vec3::NEG_X,
        Vec3::X,
    ];

    // Vanilla narrows to `float` before comparing, so the tie-breaking is
    // decided at `f32` and this has to be too.
    let direction = direction.as_vec3();
    let mut best = Vec3::NEG_Z;
    let mut best_dot = f32::MIN_POSITIVE;
    for candidate in CANDIDATES {
        let dot = direction.dot(candidate);
        if dot > best_dot {
            best_dot = dot;
            best = candidate;
        }
    }
    best
}

/// `AABB.clip(Iterable<AABB>, from, to, pos)` (`AABB.java:302`).
///
/// Divergence 6: the boxes are not sorted and not compared by distance. Vanilla
/// carries one running `scaleReference`, starting at `1.0`, and each box may
/// only lower it -- so the ordering is a running minimum and the box list's own
/// order cannot change the answer. A hit is accepted only for `0 < s < best`,
/// which is why a segment starting exactly on a face is not a hit here and is
/// left to the inside probe above.
///
/// (The `distanceToSqr` comparison in `BlockGetter.clip` orders the *block*
/// result against the *fluid* one. An arrow's `ClipContext` uses `Fluid.NONE`,
/// so the fluid shape is always empty and that comparison always picks the
/// block. hyperion clips no fluids, so it is not reproduced.)
fn clip_boxes(boxes: &[Aabb], block: IVec3, from: DVec3, diff: DVec3) -> Option<BlockHit> {
    let offset = block.as_dvec3();
    let mut best = 1.0_f64;
    let mut normal: Option<Vec3> = None;

    for shape in boxes {
        let min = shape.min.as_dvec3() + offset;
        let max = shape.max.as_dvec3() + offset;

        for axis in 0..3 {
            // An axis the segment barely moves along is skipped outright rather
            // than divided by: `dx > 1.0E-7` / `dx < -1.0E-7`, with the band
            // between them belonging to neither branch.
            let (face, outward) = if diff[axis] > 1.0E-7 {
                (min[axis], -1.0)
            } else if diff[axis] < -1.0E-7 {
                (max[axis], 1.0)
            } else {
                continue;
            };

            let s = (face - from[axis]) / diff[axis];
            if !(s > 0.0 && s < best) {
                continue;
            }

            let point = from + diff * s;
            let within = (0..3).all(|other| {
                other == axis
                    || (min[other] - 1.0E-7 < point[other] && point[other] < max[other] + 1.0E-7)
            });
            if !within {
                continue;
            }

            best = s;
            let mut face_normal = Vec3::ZERO;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "an axis index, and the normal is a unit vector"
            )]
            {
                face_normal[axis] = outward as f32;
            }
            normal = Some(face_normal);
        }
    }

    let normal = normal?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a fraction of one segment, reported at the precision the caller works in"
    )]
    Some(BlockHit {
        time: best as f32,
        block,
        point: (from + diff * best).as_vec3(),
        normal,
        inside: false,
    })
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

    /// Divergence 4. `VoxelShape.clip` probes `from + diff * 0.001` and, when
    /// that is solid, reports the hit *there* -- not at `from` -- with the face
    /// the segment would have entered by and `isInside` set.
    ///
    /// The version this replaced reported `from` itself with a zero normal,
    /// which is what a slab test clamped to `t == 0` produces. Two different
    /// points and two different faces, for the commonest case there is: an
    /// arrow loosed by a player standing in a doorway.
    #[test]
    fn a_segment_starting_inside_a_block_is_stopped_at_the_probe_point() {
        let world = cubes([IVec3::ZERO]);
        let from = Vec3::splat(0.5);
        let to = Vec3::new(10.5, 0.5, 0.5);
        let hit =
            first_block_hit(from, to, &world).expect("a shot from inside a wall does not get out");

        assert_eq!(hit.block, IVec3::ZERO);
        assert!(hit.inside, "the segment began inside the shape");
        // A thousandth of the way along, which for this ten-block segment is
        // one hundredth of a block.
        assert!((hit.time - 0.001).abs() < 1e-6, "time: {}", hit.time);
        assert!(
            (hit.point - from.lerp(to, 0.001)).length() < 1e-5,
            "point: {}",
            hit.point
        );
        // Travelling +X, so the face it would have come in through is the one
        // facing -X.
        assert_eq!(hit.normal, Vec3::NEG_X);
    }

    /// Divergence 4 again, at the boundary the probe exists to move.
    ///
    /// A segment starting exactly on a block's face is *not* inside it: the
    /// probe has already moved a thousandth of the way in, and vanilla's
    /// `findIndex` puts a coordinate sitting on a face in the cell above it.
    /// So this is an ordinary crossing, reported at `t == 0` by neither of us.
    #[test]
    fn a_segment_starting_exactly_on_a_face_is_not_inside_it() {
        let world = cubes([IVec3::new(1, 0, 0)]);
        // Starts on the wall's -X face, heading away from it.
        let hit = first_block_hit(Vec3::new(1.0, 0.5, 0.5), Vec3::new(0.0, 0.5, 0.5), &world);
        assert_eq!(hit, None, "a segment leaving a face does not hit it");

        // And heading into it: the probe lands inside, so this one is.
        let hit = first_block_hit(Vec3::new(1.0, 0.5, 0.5), Vec3::new(2.0, 0.5, 0.5), &world)
            .expect("a segment entering the wall meets it");
        assert!(hit.inside, "the probe point is within the cube");
        assert_eq!(hit.block, IVec3::new(1, 0, 0));
    }

    /// Divergence 5: `VoxelShape.clip` refuses a segment whose squared length
    /// is under `1.0E-7`, even though the traversal above walked it happily.
    ///
    /// About 3.2e-4 blocks. Short enough that no projectile produces one, and
    /// exactly the reason it is worth transcribing rather than reasoning
    /// about: a caller that samples a resting entity gets vanilla's answer
    /// instead of an arbitrary one.
    #[test]
    fn a_segment_too_short_to_clip_is_not_clipped() {
        let world = cubes([IVec3::ZERO]);
        let from = Vec3::splat(0.5);

        // Squared length 3 * (1e-4)^2 = 3e-8, under the bound.
        let inside_bound = from + Vec3::splat(1e-4);
        assert_eq!(first_block_hit(from, inside_bound, &world), None);

        // Squared length 3 * (1e-3)^2 = 3e-6, over it, and the segment is
        // inside the cube, so it hits.
        let over_bound = from + Vec3::splat(1e-3);
        let hit = first_block_hit(from, over_bound, &world)
            .expect("a segment over the length bound is clipped");
        assert!(hit.inside);
    }

    /// Divergence 6: `AABB.clip` accepts a face only for `0 < s < best`, where
    /// `best` starts at `1.0`.
    ///
    /// Both ends are strict, and both matter. A segment that reaches a face
    /// exactly at its far end has `s == 1.0` and is *not* a hit -- it is a hit
    /// on the next tick. A segment starting exactly on a face has `s == 0.0`
    /// and is not a hit either; the inside probe is what decides that case.
    #[test]
    fn a_face_reached_exactly_at_the_end_of_the_segment_is_not_hit_yet() {
        let world = cubes([IVec3::new(1, 0, 0)]);
        // Ends exactly on the wall's near face.
        assert_eq!(
            first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.0, 0.5, 0.5), &world),
            None
        );
        // A hair further, and it is a hit.
        let hit = first_block_hit(Vec3::new(0.5, 0.5, 0.5), Vec3::new(1.001, 0.5, 0.5), &world)
            .expect("a segment that crosses the face meets it");
        assert_eq!(hit.block, IVec3::new(1, 0, 0));
        assert!(!hit.inside);
        assert_eq!(hit.normal, Vec3::NEG_X);
    }

    /// Divergence 1: the traversal walks endpoints pushed outward by 1e-7 of
    /// the segment, so a start sitting exactly on a face begins in the cell
    /// *below* that face rather than the one above it.
    ///
    /// The consequence is which cells get asked for shapes, and that is not
    /// bookkeeping: a block's collision shape is not confined to its own cell.
    /// A fence post is 1.5 blocks tall and a big dripleaf's stem starts at
    /// -0.25, so a cell the segment only clips the corner of can still be the
    /// one holding the box it hits.
    #[test]
    fn the_walk_starts_below_a_face_it_begins_exactly_on() {
        let asked = std::cell::RefCell::new(Vec::new());
        let record = |block: IVec3| {
            asked.borrow_mut().push(block);
            None::<Aabb>
        };
        // Starts exactly on the x == 1 face, heading +X.
        first_block_hit(Vec3::new(1.0, 0.5, 0.5), Vec3::new(3.0, 0.5, 0.5), record);

        let asked = asked.into_inner();
        assert_eq!(
            asked.first().copied(),
            Some(IVec3::new(0, 0, 0)),
            "the walk asked about {asked:?}"
        );
    }

    /// `Direction.getApproximateNearest` maximises the dot product over
    /// `DOWN, UP, NORTH, SOUTH, WEST, EAST` with a strict `>`, so an exact
    /// diagonal resolves to the earliest of the tied directions rather than to
    /// whichever one an implementation happened to check last.
    ///
    /// Reached through the inside probe, which is the only thing that reports
    /// it.
    #[test]
    fn a_tied_diagonal_resolves_the_way_vanillas_direction_order_does() {
        let world = cubes([IVec3::ZERO]);
        // Equal -Y and -Z, no X. Vanilla checks DOWN first, so DOWN wins the
        // tie and the reported face is its opposite, UP.
        let from = Vec3::splat(0.5);
        let hit = first_block_hit(from, from + Vec3::new(0.0, -1.0, -1.0), &world)
            .expect("a segment inside the cube hits it");
        assert!(hit.inside);
        assert_eq!(hit.normal, Vec3::Y);
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
