use std::fmt::Debug;

use geometry::aabb::Aabb;

use crate::{
    Bvh, ELEMENTS_TO_ACTIVATE_LEAF, VOLUME_TO_ACTIVATE_LEAF, node::BvhNode, sort_by_largest_axis,
    utils, utils::GetAabb,
};

/// Used to find the start addresses of the elements and nodes arrays
#[derive(Debug)]
pub struct StartAddresses<T> {
    /// Base pointer to the start of the elements array
    pub start_elements_ptr: *const T,
    pub start_nodes_ptr: *const BvhNode,
}

impl<T> StartAddresses<T> {
    const fn element_start_index(&self, slice: &[T]) -> isize {
        unsafe { slice.as_ptr().offset_from(self.start_elements_ptr) }
    }

    const fn node_start_index(&self, slice: &[BvhNode]) -> isize {
        unsafe { slice.as_ptr().offset_from(self.start_nodes_ptr) }
    }
}

unsafe impl<T: Send> Send for StartAddresses<T> {}
unsafe impl<T: Sync> Sync for StartAddresses<T> {}

impl<T> Bvh<T>
where
    T: Debug + Send + Copy + Sync,
{
    #[tracing::instrument(skip_all, fields(elements_len = elements.len()))]
    pub fn build(mut elements: Vec<T>, get_aabb: impl GetAabb<T> + Sync) -> Self {
        let max_threads = utils::thread_count_pow2();

        let len = elements.len();

        // // 1.7 works too, 2.0 is upper bound ... 1.8 is probably best
        // todo: make this more mathematically derived
        let capacity = ((len / ELEMENTS_TO_ACTIVATE_LEAF) as f64 * 8.0) as usize;

        // [A]
        let capacity = capacity.max(16);

        let mut nodes = vec![BvhNode::DUMMY; capacity];

        let bvh = StartAddresses {
            start_elements_ptr: elements.as_ptr(),
            start_nodes_ptr: nodes.as_ptr(),
        };

        #[expect(
            clippy::indexing_slicing,
            reason = "Look at [A]. The length is at least 16, so this is safe."
        )]
        let nodes_slice = &mut nodes[1..];

        let (root, _) = build_in(&bvh, &mut elements, max_threads, 0, nodes_slice, &get_aabb);

        Self {
            nodes,
            elements,
            root,
        }
    }
}

#[allow(clippy::float_cmp)]
pub fn build_in<T>(
    addresses: &StartAddresses<T>,
    elements: &mut [T],
    max_threads: usize,
    nodes_idx: usize,
    nodes: &mut [BvhNode],
    get_aabb: &(impl GetAabb<T> + Sync),
) -> (i32, usize)
where
    T: Send + Copy + Sync + Debug,
{
    // aabb that encompasses all elements
    let aabb: Aabb = elements.iter().map(get_aabb).collect();
    let volume = aabb.volume();

    if elements.len() <= ELEMENTS_TO_ACTIVATE_LEAF || volume <= VOLUME_TO_ACTIVATE_LEAF {
        let idx_start = addresses.element_start_index(elements);

        let node = BvhNode::create_leaf(aabb, idx_start as usize, elements.len());

        let set = &mut nodes[nodes_idx..=nodes_idx];
        set[0] = node;

        let idx = addresses.node_start_index(set);

        let idx = idx as i32;

        debug_assert!(idx > 0);

        return (idx, nodes_idx + 1);
    }

    sort_by_largest_axis(elements, &aabb, get_aabb);

    let element_split_idx = elements.len() / 2;

    let (left_elems, right_elems) = elements.split_at_mut(element_split_idx);

    debug_assert!(max_threads != 0);

    let original_node_idx = nodes_idx;

    let (left, right, nodes_idx, to_set) = if max_threads == 1 {
        let start_idx = nodes_idx;
        let (left, nodes_idx) = build_in(
            addresses,
            left_elems,
            max_threads,
            nodes_idx + 1,
            nodes,
            get_aabb,
        );

        let (right, nodes_idx) = build_in(
            addresses,
            right_elems,
            max_threads,
            nodes_idx,
            nodes,
            get_aabb,
        );
        let end_idx = nodes_idx;

        debug_assert!(start_idx != end_idx);

        (
            left,
            right,
            nodes_idx,
            &mut nodes[original_node_idx..=original_node_idx],
        )
    } else {
        let max_threads = max_threads >> 1;

        let (to_set, nodes) = nodes.split_at_mut(1);

        let node_split_idx = nodes.len() / 2;
        let (left_nodes, right_nodes) = match true {
            true => {
                let (left, right) = nodes.split_at_mut(node_split_idx);
                (left, right)
            }
            false => {
                let (right, left) = nodes.split_at_mut(node_split_idx);
                (left, right)
            }
        };

        let (left, right) = rayon::join(
            || build_in(addresses, left_elems, max_threads, 0, left_nodes, get_aabb),
            || {
                build_in(
                    addresses,
                    right_elems,
                    max_threads,
                    0,
                    right_nodes,
                    get_aabb,
                )
            },
        );

        (left.0, right.0, 0, to_set)
    };

    let node = BvhNode { aabb, left, right };

    to_set[0] = node;
    let idx = unsafe { to_set.as_ptr().offset_from(addresses.start_nodes_ptr) };
    let idx = idx as i32;

    // trace!("internal nodes_idx {:03}", original_node_idx);

    debug_assert!(idx > 0);

    (idx, nodes_idx + 1)
}
