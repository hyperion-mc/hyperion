//! A bounding volume hierarchy (BVH) implementation optimized for spatial queries.
//!
//! This crate provides a BVH data structure that organizes objects in 3D space for efficient
//! spatial queries like collision detection and ray casting. The BVH recursively subdivides
//! space into smaller regions, grouping nearby objects together.
//!
//! The implementation uses a binary tree structure where:
//! - Internal nodes contain axis-aligned bounding boxes (AABBs) that fully enclose their children
//! - Leaf nodes contain the actual geometric objects
//! - The tree is built top-down by recursively splitting along the largest axis

#![feature(portable_simd)]
#![feature(gen_blocks)]
#![feature(coroutines)]
#![allow(clippy::redundant_pub_crate, clippy::pedantic)]

use std::fmt::Debug;

use arrayvec::ArrayVec;
use geometry::aabb::Aabb;

/// Maximum number of elements allowed in a leaf node before splitting
const ELEMENTS_TO_ACTIVATE_LEAF: usize = 16;

/// Maximum volume of a node's bounding box before splitting
const VOLUME_TO_ACTIVATE_LEAF: f32 = 5.0;

mod node;
use node::BvhNode;

mod build;
mod query;
mod utils;

#[cfg(feature = "plot")]
pub mod plot;

/// A bounding volume hierarchy that organizes objects in 3D space.
///
/// The BVH stores elements of type `T` in a tree structure for efficient spatial queries.
/// Each node in the tree has an axis-aligned bounding box that fully contains all elements
/// in its subtree.
#[derive(Debug, Clone)]
pub struct Bvh<T> {
    /// The nodes making up the BVH tree structure
    nodes: Vec<BvhNode>,
    /// The actual elements being stored
    elements: Vec<T>,
    /// Index of the root node
    root: i32,
}

impl<T> Default for Bvh<T> {
    fn default() -> Self {
        Self {
            nodes: vec![BvhNode::DUMMY],
            elements: Vec::new(),
            root: 0,
        }
    }
}

impl<T> Bvh<T> {
    /// Clears the BVH, removing all elements and nodes.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl<T> Bvh<T> {
    /// Returns a reference to the root node of the BVH.
    fn root(&self) -> Node<'_, T> {
        let root = self.root;
        if root < 0 {
            return Node::Leaf(&self.elements[..]);
        }

        Node::Internal(&self.nodes[root as usize])
    }
}

/// A trait for implementing different node splitting strategies.
pub trait Heuristic {
    /// Determines where to split a set of elements.
    ///
    /// Returns the index at which to split the elements into left and right groups.
    fn heuristic<T>(elements: &[T]) -> usize;
}

/// A simple splitting heuristic that divides elements in half.
pub struct TrivialHeuristic;

impl Heuristic for TrivialHeuristic {
    fn heuristic<T>(elements: &[T]) -> usize {
        elements.len() / 2
    }
}

/// Sorts elements by their position along the largest axis of their bounding box.
///
/// Returns which axis was used for sorting (0 = x, 1 = y, 2 = z).
fn sort_by_largest_axis<T>(elements: &mut [T], aabb: &Aabb, get_aabb: &impl Fn(&T) -> Aabb) -> u8 {
    let lens = aabb.lens();
    let largest = lens.x.max(lens.y).max(lens.z);

    let len = elements.len();
    let median_idx = len / 2;

    #[expect(
        clippy::float_cmp,
        reason = "we are not modifying; we are comparing exact values"
    )]
    let key = if lens.x == largest {
        0_u8
    } else if lens.y == largest {
        1
    } else {
        2
    };

    elements.select_nth_unstable_by(median_idx, |a, b| {
        let a = get_aabb(a).min.as_ref()[key as usize];
        let b = get_aabb(b).min.as_ref()[key as usize];

        unsafe { a.partial_cmp(&b).unwrap_unchecked() }
    });

    key
}

/// A node in the BVH tree.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Node<'a, T> {
    /// An internal node containing child nodes
    Internal(&'a BvhNode),
    /// A leaf node containing actual elements
    Leaf(&'a [T]),
}

impl BvhNode {
    /// A dummy node used as a placeholder.
    pub const DUMMY: Self = Self {
        aabb: Aabb::NULL,
        left: 0,
        right: 0,
    };

    /// Returns a reference to the left child node if it exists.
    fn left<'a, T>(&self, root: &'a Bvh<T>) -> Option<&'a Self> {
        let left = self.left;

        if left < 0 {
            return None;
        }

        root.nodes.get(left as usize)
    }

    /// Processes the children of this node with different callbacks for internal nodes and leaves.
    #[allow(unused)]
    fn switch_children<'a, T>(
        &'a self,
        root: &'a Bvh<T>,
        mut process_children: impl FnMut(&'a Self),
        mut process_leaf: impl FnMut(&'a [T]),
    ) {
        let left_idx = self.left;

        if left_idx < 0 {
            let start_idx = -left_idx - 1;
            // let start_idx = usize::try_from(start_idx).expect("failed to convert index");

            let start_idx = start_idx as usize;

            let len = self.right;

            let elems = &root.elements[start_idx..start_idx + len as usize];
            process_leaf(elems);
        } else {
            let left = unsafe { self.left(root).unwrap_unchecked() };
            let right = unsafe { self.right(root) };

            process_children(left);
            process_children(right);
        }
    }

    /// Returns an iterator over this node's children.
    fn children<'a, T>(&'a self, root: &'a Bvh<T>) -> impl Iterator<Item = Node<'a, T>> {
        self.children_vec(root).into_iter()
    }

    /// Returns a vector containing this node's children.
    fn children_vec<'a, T>(&'a self, root: &'a Bvh<T>) -> ArrayVec<Node<'a, T>, 2> {
        let left = self.left;

        // leaf
        if left < 0 {
            let start_idx = left.checked_neg().expect("failed to negate index") - 1;

            let start_idx = usize::try_from(start_idx).expect("failed to convert index");

            let len = self.right as usize;

            let elems = &root.elements[start_idx..start_idx + len];
            let mut vec = ArrayVec::new();
            vec.push(Node::Leaf(elems));
            return vec;
        }

        let mut vec = ArrayVec::new();
        if let Some(left) = self.left(root) {
            vec.push(Node::Internal(left));
        }

        let right = unsafe { self.right(root) };
        vec.push(Node::Internal(right));

        vec
    }

    /// Returns a reference to the right child node.
    ///
    /// # Safety
    /// Only safe to call if the left child exists. If left exists then right must also exist.
    unsafe fn right<'a, T>(&self, root: &'a Bvh<T>) -> &'a Self {
        let right = self.right;

        debug_assert!(right > 0);

        &root.nodes[right as usize]
    }
}

#[cfg(test)]
mod tests;
