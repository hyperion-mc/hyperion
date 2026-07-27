use std::{alloc::Allocator, ops::Range};

use arrayvec::ArrayVec;
use bytes::Bytes;
use glam::I16Vec2;
use heapless::binary_heap::Min;

use crate::{Bvh, ROOT_IDX, aabb::Aabb, child_left, child_right, node::Expanded};

const DFS_STACK_SIZE: usize = 32;
const HEAP_SIZE: usize = 32;

impl<A: Allocator> Bvh<Bytes, A> {
    pub fn get_closest_slice_bytes(&self, input: I16Vec2) -> Option<Bytes> {
        let idx = self.get_closest(input)?;
        let idx = idx.start as usize..idx.end as usize;
        Some(self.data.slice(idx))
    }

    pub fn get_in_slices_bytes(&self, query: Aabb) -> ArrayVec<Bytes, DFS_STACK_SIZE> {
        self.get_in(query)
            .into_iter()
            .map(|range| self.data.slice(range.start as usize..range.end as usize))
            .collect()
    }
}

impl<T, A: Allocator> Bvh<Vec<T>, A> {
    pub fn get_closest_slice(&self, input: I16Vec2) -> Option<&[T]> {
        let idx = self.get_closest(input)?;
        let idx = idx.start as usize..idx.end as usize;
        Some(&self.data[idx])
    }

    pub fn get_in_slices(&self, query: Aabb) -> ArrayVec<&[T], DFS_STACK_SIZE> {
        self.get_in(query)
            .into_iter()
            .map(|range| &self.data[range.start as usize..range.end as usize])
            .collect()
    }
}

pub trait Len {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Len for Vec<T> {
    fn len(&self) -> usize {
        self.len()
    }
}

impl Len for Bytes {
    fn len(&self) -> usize {
        self.len()
    }
}

impl Bvh<Vec<u8>> {
    #[must_use]
    pub fn into_bytes(self) -> Bvh<Bytes> {
        Bvh {
            nodes: self.nodes,
            data: self.data.into(),
            leaves: self.leaves,
        }
    }
}

impl<L: Len, A: Allocator> Bvh<L, A> {
    /// # Panics
    /// If there are too many elements that overflow `HEAP_SIZE`
    #[allow(clippy::too_many_lines)]
    pub fn get_closest(&self, input: I16Vec2) -> Option<Range<u32>> {
        #[derive(Debug, Copy, Clone)]
        struct MinNode {
            dist2: u32,
            expanded: Expanded,
            idx: u32,
        }

        impl PartialEq for MinNode {
            fn eq(&self, other: &Self) -> bool {
                self.dist2 == other.dist2
            }
        }

        impl Eq for MinNode {}

        impl Ord for MinNode {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.dist2.cmp(&other.dist2)
            }
        }

        impl PartialOrd for MinNode {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        let mut max_distance_to_closest = u32::MAX;

        let mut new_node = |idx: u32, expanded: Expanded| match expanded {
            Expanded::Aabb(aabb) => {
                let (dist2_min, dist2_max) = aabb.min_max_distance2(input);

                if max_distance_to_closest < dist2_min {
                    return None;
                }

                if dist2_max < max_distance_to_closest {
                    max_distance_to_closest = dist2_max;
                }

                Some(MinNode {
                    dist2: dist2_min,
                    expanded,
                    idx,
                })
            }
            Expanded::Leaf(leaf) => {
                if leaf.is_invalid() {
                    return None;
                }

                #[allow(clippy::cast_sign_loss)]
                let difference = (leaf.point.as_ivec2() - input.as_ivec2()).abs().as_uvec2();
                let dist2 = difference.length_squared();

                if max_distance_to_closest < dist2 {
                    return None;
                }

                if dist2 < max_distance_to_closest {
                    max_distance_to_closest = dist2;
                }

                Some(MinNode {
                    dist2,
                    expanded,
                    idx,
                })
            }
        };

        if self.data.is_empty() {
            return None;
        }

        let mut heap: heapless::BinaryHeap<MinNode, Min, HEAP_SIZE> = heapless::BinaryHeap::new();

        // root idx
        let node = unsafe { self.get_node(ROOT_IDX) };
        let expanded_node = node.into_expanded().expect("root node is always valid");

        if let Expanded::Leaf(leaf) = expanded_node {
            // if root node is a leaf, return the leaf

            let ptr = leaf.ptr;
            let start = self.leaves[ptr as usize].element_index;
            let end = self.leaves[ptr as usize + 1].element_index;

            return Some(start..end);
        }

        let dist2 = u32::MAX;

        heap.push(MinNode {
            dist2,
            expanded: node.into_expanded().expect("root node is always valid"),
            idx: ROOT_IDX,
        })
        .unwrap();

        while let Some(context) = heap.pop() {
            match context.expanded {
                Expanded::Leaf(leaf) => {
                    let ptr = leaf.ptr;
                    let start = self.leaves[ptr as usize].element_index;
                    let end = self.leaves[ptr as usize + 1].element_index;

                    return Some(start..end);
                }
                Expanded::Aabb(..) => {
                    let left = child_left(context.idx);

                    let node = unsafe { self.get_node(left) };

                    if let Some(node) = node.into_expanded()
                        && let Some(node) = new_node(left, node)
                    {
                        heap.push(node).unwrap();
                    }

                    let right = child_right(context.idx);
                    let node = unsafe { self.get_node(right) };
                    if let Some(node) = node.into_expanded()
                        && let Some(node) = new_node(right, node)
                    {
                        heap.push(node).unwrap();
                    }
                }
            }
        }

        None
    }

    pub fn get_in(&self, query: Aabb) -> Vec<Range<u32>> {
        let mut to_send_indices: Vec<Range<u32>> = Vec::new();

        if self.data.is_empty() {
            // nothing
            return to_send_indices;
        }

        let mut dfs_stack: Vec<u32> = Vec::new();

        // so we do not need special case (there is always a last)
        to_send_indices.push(0..0);
        dfs_stack.push(ROOT_IDX);

        while let Some(idx) = dfs_stack.pop() {
            let node = unsafe { self.get_node(idx) };

            match node.into_expanded() {
                Some(Expanded::Leaf(leaf)) => {
                    if !query.contains_point(leaf.point) {
                        continue;
                    }

                    let ptr = leaf.ptr;

                    let start = unsafe { self.leaves.get_unchecked(ptr as usize) }.element_index;
                    let end = unsafe { self.leaves.get_unchecked(ptr as usize + 1) }.element_index;

                    let last = unsafe { to_send_indices.last_mut().unwrap_unchecked() };

                    if last.end == start {
                        // combine
                        last.end = end;
                    } else {
                        to_send_indices.push(start..end);
                    }
                }
                Some(Expanded::Aabb(aabb)) => {
                    if !aabb.intersects(query) {
                        continue;
                    }

                    let left = child_left(idx);
                    let right = child_right(idx);

                    dfs_stack.push(right);

                    // we want to do left first because this is how we are doing DFS when building the tree
                    // if we do not do this in the right order dfs_stack will be in the wrong order
                    dfs_stack.push(left);
                }
                None => {}
            }
        }

        if unsafe { to_send_indices.get_unchecked(0) }.end == 0 {
            // todo: more efficient?
            to_send_indices.remove(0);
        }

        to_send_indices
    }
}
