use std::fmt::Debug;

use arrayvec::ArrayVec;
use geometry::aabb::Aabb;

use crate::{Bvh, Node, utils::GetAabb};

impl<T: Debug> Bvh<T> {
    /// Returns an iterator over elements whose bounding boxes overlap with the target region.
    ///
    /// The iterator yields references to elements whose bounding boxes (obtained via `get_aabb`)
    /// collide with the target [`Aabb`].
    pub fn range<'a>(
        &'a self,
        target: Aabb,
        get_aabb: impl GetAabb<T> + 'a,
    ) -> impl Iterator<Item = &'a T> + 'a {
        CollisionIter::new(self, target, get_aabb)
    }
}

pub struct CollisionIter<'a, T, F> {
    bvh: &'a Bvh<T>,
    target: Aabb,
    get_aabb: F,
    stack: ArrayVec<Node<'a, T>, 64>,
    current_leaf: Option<(&'a [T], usize)>,
}

impl<'a, T, F> CollisionIter<'a, T, F>
where
    F: GetAabb<T>,
{
    fn new(bvh: &'a Bvh<T>, target: Aabb, get_aabb: F) -> Self {
        let mut stack = ArrayVec::new();
        // Initialize stack with root if it collides
        match bvh.root() {
            Node::Internal(root) => {
                if root.aabb.collides(&target) {
                    stack.push(Node::Internal(root));
                }
            }
            Node::Leaf(leaf) => {
                // We'll handle collision checks in next() as we iterate through leaves
                stack.push(Node::Leaf(leaf));
            }
        }

        Self {
            bvh,
            target,
            get_aabb,
            stack,
            current_leaf: None,
        }
    }
}

impl<'a, T, F> Iterator for CollisionIter<'a, T, F>
where
    F: GetAabb<T>,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If we're currently iterating over a leaf's elements
            if let Some((leaf, index)) = &mut self.current_leaf {
                if *index < leaf.len() {
                    let elem = &leaf[*index];
                    *index += 1;

                    let elem_aabb = (self.get_aabb)(elem);
                    if elem_aabb.collides(&self.target) {
                        return Some(elem);
                    }
                    // If not colliding, continue to next element in leaf
                    continue;
                } else {
                    // Leaf exhausted
                    self.current_leaf = None;
                }
            }

            // If no current leaf, pop from stack
            let node = self.stack.pop()?;
            match node {
                Node::Internal(internal) => {
                    // Push children that potentially collide
                    for child in internal.children(self.bvh) {
                        match child {
                            Node::Internal(child_node) => {
                                if child_node.aabb.collides(&self.target) {
                                    self.stack.push(Node::Internal(child_node));
                                }
                            }
                            Node::Leaf(child_leaf) => {
                                // We'll check collisions inside the leaf iteration
                                self.stack.push(Node::Leaf(child_leaf));
                            }
                        }
                    }
                }
                Node::Leaf(leaf) => {
                    // Start iterating over this leaf's elements
                    self.current_leaf = Some((leaf, 0));
                }
            }
        }
    }
}
