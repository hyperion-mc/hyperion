// #![expect(clippy::print_stdout)]
//
// use std::collections::HashSet;
//
// use geometry::aabb::CheckableAabb;
// use itertools::Itertools;
//
// use super::*;
//
// fn collisions_naive(
//     elements: &[Aabb],
//     target: Aabb,
// ) -> Result<HashSet<CheckableAabb>, ordered_float::FloatIsNan> {
//     elements
//         .iter()
//         .filter(move |elem| elem.collides(&target))
//         .copied()
//         .map(CheckableAabb::try_from)
//         .try_collect()
// }
//
// #[test]
// fn test_build_all_sizes() {
//     tracing_subscriber::fmt::init();
//
//     let counts = &[0, 1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];
//
//     for count in counts {
//         let elements = create_random_elements_1(*count, 100.0);
//         Bvh::build::<TrivialHeuristic>(elements);
//     }
// }
//
// #[test]
// fn test_query_one() {
//     let elements = create_random_elements_1(10_000, 100.0);
//
//     let elems = elements.clone();
//     let bvh = Bvh::build::<TrivialHeuristic>(elems);
//
//     let element = random_aabb(30.0);
//
//     println!("element: {}", element);
//
//     let naive_collisions = collisions_naive(&elements, element).unwrap();
//
//     let mut num_collisions = 0;
//
//     let mut bvh_collisions = Vec::new();
//     // 1000 x 1000 x 1000 = 1B ... 1B / 1M = 1000 blocks on average...
//     // on average num_collisions should be super low
//     bvh.get_collisions(element, |elem| {
//         num_collisions += 1;
//         assert!(elem.collides(&element));
//         bvh_collisions.push(CheckableAabb::try_from(*elem).unwrap());
//         true
//     });
//
//     for elem in &naive_collisions {
//         assert!(bvh_collisions.contains(elem));
//     }
//
//     assert_eq!(num_collisions, naive_collisions.len());
//
//     // bvh.plot("test.png").unwrap()
// }
//
// #[test]
// fn test_query_all() {
//     let elements = create_random_elements_1(10_000, 100.0);
//     let bvh = Bvh::build::<TrivialHeuristic>(elements);
//
//     let node_count = bvh.nodes.len();
//     println!("node count: {}", node_count);
//
//     let mut num_collisions = 0;
//
//     bvh.get_collisions(Aabb::EVERYTHING, |_| {
//         num_collisions += 1;
//         true
//     });
//
//     assert_eq!(num_collisions, 10_000);
// }
//
// #[test]
// fn children_returns_empty_leaf_when_no_children() {
//     let node = BvhNode {
//         aabb: Aabb::NULL,
//         left: -1,
//         right: 0,
//     };
//     let bvh: Bvh<i32> = Bvh {
//         nodes: vec![BvhNode::DUMMY, node],
//         elements: Vec::new(),
//         root: 1,
//     };
//     assert_eq!(node.children(&bvh).next(), Some(Node::Leaf(&[])));
// }
//
// #[test]
// fn children_returns_internal_nodes() {
//     let aabb = random_aabb(100.0);
//
//     let child_node = BvhNode::EMPTY_LEAF;
//
//     let node = BvhNode {
//         aabb: aabb.expand(10.0),
//         left: 1,
//         right: 2,
//     };
//
//     let bvh: Bvh<i32> = Bvh {
//         nodes: vec![BvhNode::DUMMY, child_node, child_node],
//         elements: Vec::new(),
//         root: 42, // root does not matter in this case
//     };
//     let mut children = node.children(&bvh);
//     assert_eq!(children.next(), Some(Node::Internal(&child_node)));
//     assert_eq!(children.next(), Some(Node::Internal(&child_node)));
//     assert!(children.next().is_none());
// }
//
// #[test]
// fn get_closest_returns_closest_element() {
//     let elements = vec![
//         Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)),
//         Aabb::new(Vec3::new(2.0, 2.0, 2.0), Vec3::new(3.0, 3.0, 3.0)),
//         Aabb::new(Vec3::new(4.0, 4.0, 4.0), Vec3::new(5.0, 5.0, 5.0)),
//     ];
//     let bvh = Bvh::build::<TrivialHeuristic>(elements);
//
//     let target = Vec3::new(2.5, 2.5, 2.5);
//     let closest = bvh.get_closest(target);
//
//     assert!(closest.is_some());
//     let (closest_element, _) = closest.unwrap();
//     assert_eq!(
//         closest_element.aabb(),
//         Aabb::new(Vec3::new(2.0, 2.0, 2.0), Vec3::new(3.0, 3.0, 3.0))
//     );
// }
//
// #[test]
// fn get_closest_returns_closest_element_with_random_data() {
//     fastrand::seed(7);
//
//     // todo: this might actually fail in some cases. it failed in CI when not using a seed; I added EPSILON, might fix.
//
//     let elements: Vec<Aabb> = (0..1000)
//         .map(|_| {
//             let min = Vec3::new(
//                 fastrand::f32().mul_add(200.0, -100.0),
//                 fastrand::f32().mul_add(200.0, -100.0),
//                 fastrand::f32().mul_add(200.0, -100.0),
//             );
//             let max = min + Vec3::new(1.0, 1.0, 1.0);
//             Aabb::new(min, max)
//         })
//         .collect();
//
//     let bvh = Bvh::build::<TrivialHeuristic>(elements.clone());
//
//     let target = Vec3::new(
//         fastrand::f32().mul_add(200.0, -100.0),
//         fastrand::f32().mul_add(200.0, -100.0),
//         fastrand::f32().mul_add(200.0, -100.0),
//     );
//     let closest = bvh.get_closest(target);
//
//     assert!(closest.is_some());
//     let (closest_element, _) = closest.unwrap();
//
//     // Check that the closest element is indeed the closest by comparing with all elements
//     for element in &elements {
//         assert!(
//             element.aabb().dist2(target) >= closest_element.aabb().dist2(target) - f32::EPSILON
//         );
//     }
// }
//
// #[test]
// fn get_closest_returns_none_when_no_elements() {
//     let elements: Vec<Aabb> = vec![];
//     let bvh = Bvh::build::<TrivialHeuristic>(elements);
//
//     let target = Vec3::new(2.5, 2.5, 2.5);
//     let closest = bvh.get_closest(target);
//
//     assert!(closest.is_none());
// }

use geometry::{aabb::Aabb, ray::Ray};
use glam::Vec3;

use crate::Bvh;

/// A ray query on an empty index returns nothing rather than crashing.
///
/// The spatial index is `Bvh::default()` on the first tick, before its
/// `OnStore` rebuild has run, and a projectile loosed along a non-axis-aligned
/// direction casts a ray with every component non-zero. That ray intersects
/// the `NULL` root box, and while the empty tree was the `DUMMY` internal node
/// rather than an empty leaf the traversal followed a phantom right child: a
/// debug assert here, an infinite loop with it compiled out. An axis-aligned
/// ray missed the box by luck and hid it.
#[test]
fn empty_bvh_ray_query_is_none() {
    fn no_aabb(_: &u32) -> Aabb {
        unreachable!("an empty tree has no elements to measure")
    }

    let diagonal = Ray::new(Vec3::new(0.5, 128.0, 0.5), Vec3::new(-1.6, 1.0, 2.3));

    let default: Bvh<u32> = Bvh::default();
    assert!(default.first_ray_collision(diagonal, no_aabb).is_none());

    // The built-from-empty tree must be just as safe as the default one.
    let built: Bvh<u32> = Bvh::build(Vec::new(), no_aabb);
    assert!(built.first_ray_collision(diagonal, no_aabb).is_none());
}
