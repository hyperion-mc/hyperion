use std::{cmp::Reverse, collections::BinaryHeap, fmt::Debug};

use geometry::{aabb::Aabb, ray::Ray};
use ordered_float::NotNan;

use crate::{Bvh, Node, utils::NodeOrd};

impl<T: Debug> Bvh<T> {
    /// Returns the closest element hit by the ray and the intersection distance (t) along the ray.
    ///
    /// If no element is hit, returns `None`.
    #[allow(clippy::excessive_nesting)]
    pub fn first_ray_collision(
        &self,
        ray: Ray,
        get_aabb: impl Fn(&T) -> Aabb,
    ) -> Option<(&T, NotNan<f32>)> {
        let mut closest_t = NotNan::new(f32::INFINITY).unwrap();
        let mut closest_elem = None;

        let root = self.root();

        match root {
            Node::Leaf(elems) => {
                // Only a leaf: check all elements directly.
                for elem in elems {
                    if let Some(t) = get_aabb(elem).intersect_ray(&ray)
                        && t < closest_t
                        && t.into_inner() >= 0.0
                    {
                        closest_t = t;
                        closest_elem = Some(elem);
                    }
                }
            }
            Node::Internal(internal) => {
                let mut heap: BinaryHeap<_> = BinaryHeap::new();

                // Check if the ray hits the root node's AABB
                if let Some(t) = internal.aabb.intersect_ray(&ray)
                    && t.into_inner() >= 0.0
                {
                    heap.push(Reverse(NodeOrd {
                        node: internal,
                        by: t,
                    }));
                }

                while let Some(Reverse(current)) = heap.pop() {
                    let node = current.node;
                    let node_t = current.by;

                    // If the node AABB is farther than any known intersection, prune
                    if node_t > closest_t {
                        continue;
                    }

                    for child in node.children(self) {
                        match child {
                            Node::Internal(child_node) => {
                                if let Some(t) = child_node.aabb.intersect_ray(&ray)
                                    && t < closest_t
                                    && t.into_inner() >= 0.0
                                {
                                    heap.push(Reverse(NodeOrd {
                                        node: child_node,
                                        by: t,
                                    }));
                                }
                            }
                            Node::Leaf(elems) => {
                                for elem in elems {
                                    if let Some(t) = get_aabb(elem).intersect_ray(&ray)
                                        && t < closest_t
                                        && t.into_inner() >= 0.0
                                    {
                                        closest_t = t;
                                        closest_elem = Some(elem);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        closest_elem.map(|elem| (elem, closest_t))
    }
}
