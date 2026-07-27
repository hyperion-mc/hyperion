#![feature(allocator_api)]
#![feature(associated_type_defaults)]

use std::{
    alloc::{Allocator, Global},
    cell::Cell,
    fmt::{Debug, Formatter},
    num::NonZeroU32,
};

use more_asserts::debug_assert_lt;

pub use crate::aabb::Aabb;
use crate::{
    node::{Expanded, Leaf, LeafPtr, Node},
    sealed::PointWithData,
};

mod aabb;

pub mod node;
mod print;

mod query;

pub struct Bvh<L, A: Allocator = Global> {
    nodes: Box<[Cell<Node>], A>,
    data: L,
    leaves: Vec<Leaf, A>,
}

/// A node with no valid child must itself be an invalid leaf.
#[cfg(debug_assertions)]
fn debug_assert_invalid_leaf(node: Option<Expanded>, side: &str) {
    let Some(node) = node else { return };
    let Expanded::Leaf(leaf) = node else {
        unreachable!("expected a leaf on the {side}, got {node:?}")
    };
    debug_assert!(
        leaf.is_invalid(),
        "expected invalid {side} leaf, got {leaf:?}"
    );
}

impl<L, A: Allocator> Bvh<L, A> {
    pub fn into_inner(self) -> (Vec<Leaf, A>, L) {
        (self.leaves, self.data)
    }

    pub const fn inner(&self) -> (&Vec<Leaf, A>, &L) {
        (&self.leaves, &self.data)
    }

    pub const fn inner_mut(&mut self) -> (&mut Vec<Leaf, A>, &mut L) {
        (&mut self.leaves, &mut self.data)
    }
}

impl<T: Debug, A: Allocator> Debug for Bvh<Vec<T, A>, A> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.print())
    }
}

impl<L: Default, A: Allocator + Default> Default for Bvh<L, A> {
    fn default() -> Self {
        Self {
            // zeroed so everything is equivalent to Aabb with 0,0
            nodes: Box::new_in([], A::default()),
            data: L::default(),
            leaves: Vec::with_capacity_in(0, A::default()),
        }
    }
}

// const HALF_I16_MAX: u16 = (i16::MAX / 2) as u16; // 1_073_741_823

#[must_use]
pub fn add_half_max_and_convert(x: i16) -> u16 {
    // todo: try more efficient method
    // (x as u16).wrapping_add(HALF_I16_MAX)

    let i16_min = i32::from(i16::MIN);
    let naive_result = i32::from(x) - i16_min;
    unsafe { u16::try_from(naive_result).unwrap_unchecked() }
}

pub trait Point {
    /// Generally, this will be an [`u8`]
    fn point(&self) -> glam::I16Vec2;
}

impl Point for glam::I16Vec2 {
    fn point(&self) -> glam::I16Vec2 {
        *self
    }
}

impl Point for &glam::I16Vec2 {
    fn point(&self) -> glam::I16Vec2 {
        **self
    }
}

pub trait Data {
    type Unit;
    type Context<'a>: Copy
        = ()
    where
        Self: 'a;
    fn data<'a: 'c, 'b: 'c, 'c>(&'a self, context: Self::Context<'b>) -> &'c [Self::Unit];
}

mod sealed {
    use crate::{Data, Point};

    pub trait PointWithData: Point + Data {}
}

impl<T> sealed::PointWithData for T where T: Point + Data {}

impl<T> Bvh<Vec<T>> {
    #[must_use]
    pub fn build<I>(input: &mut [I], context: I::Context<'_>) -> Self
    where
        I: PointWithData<Unit = T>,
        T: Copy + 'static,
    {
        Self::build_in(input, Global, context)
    }
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: Send, A: Allocator> Send for Bvh<T, A> {}
unsafe impl<T: Sync, A: Allocator> Sync for Bvh<T, A> {}

impl<T, A: Allocator + Clone> Bvh<Vec<T, A>, A> {
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_lines)]
    pub fn build_in<I>(input: &mut [I], alloc: A, context: I::Context<'_>) -> Self
    where
        I: PointWithData<Unit = T>,
        T: Copy + 'static,
    {
        if input.is_empty() {
            return Self {
                nodes: Box::new_in([], alloc.clone()),
                data: Vec::new_in(alloc.clone()),
                leaves: Vec::new_in(alloc),
            };
        }

        input.sort_by_cached_key(|x| {
            let point = x.point();
            let x = add_half_max_and_convert(point.x);
            let y = add_half_max_and_convert(point.y);
            fast_hilbert::xy2h(x, y, 32)
        });

        let (data, mut leaves, points) = process_input(input, alloc.clone(), context);

        let leaves_next_pow2 = leaves.len().next_power_of_two();
        let total_size = leaves_next_pow2 + leaves.len();

        let mut nodes = unsafe { Box::new_zeroed_slice_in(total_size, alloc).assume_init() };

        let mut root_set = false;

        for (i, &point) in points.iter().enumerate() {
            let leaf = Node::leaf(point, unsafe { u32::try_from(i).unwrap_unchecked() });
            let leaf = Cell::new(leaf);
            let idx = i + leaves_next_pow2;

            if idx == 1 {
                root_set = true;
            }

            nodes[idx] = leaf;
        }

        let mut current_level_start = leaves_next_pow2 / 2;

        while current_level_start >= 1 {
            for i in current_level_start..(current_level_start * 2) {
                if i == 1 {
                    root_set = true;
                }

                let i = i as u32;

                let left = child_left(i) as usize;
                let right = child_right(i) as usize;

                let left = nodes.get(left).map(Cell::get).and_then(Node::into_expanded);
                let right = nodes
                    .get(right)
                    .map(Cell::get)
                    .and_then(Node::into_expanded);

                let parent_node = match (left, right) {
                    (Some(Expanded::Aabb(left)), Some(Expanded::Aabb(right))) => {
                        let aabb = left.merge(right);
                        Node::aabb(aabb)
                    }
                    (Some(Expanded::Aabb(left)), Some(Expanded::Leaf(right))) => {
                        let aabb = left.enclose(right.point);
                        Node::aabb(aabb)
                    }
                    (Some(Expanded::Aabb(left)), ..) => {
                        // todo: try to restructure to eliminate this branch
                        Node::aabb(left)
                    }
                    (Some(Expanded::Leaf(left)), Some(Expanded::Leaf(right))) // valid, valid
                        if left.is_valid() && right.is_valid() =>
                    {
                        debug_assert!(left.point != right.point, "got {left:?} and {right:?}");
                        let aabb = Aabb::enclosing_aabb([left.point, right.point]);
                        Node::aabb(aabb)
                    }
                    (Some(Expanded::Leaf(left)), _) // valid, invalid
                        if left.is_valid() =>
                    {
                        Node::from(left)
                    }
                    (left, right) => {
                        #[cfg(debug_assertions)]
                        {
                            debug_assert_invalid_leaf(left, "left");
                            debug_assert_invalid_leaf(right, "right");
                        }
                        Node::from(LeafPtr::INVALID)
                    },
                };

                #[cfg(debug_assertions)]
                {
                    if let Some(Expanded::Leaf(leaf)) = parent_node.into_expanded() {
                        debug_assert_lt!(
                            leaf.ptr,
                            leaves.len() as u32,
                            "leaf.ptr {} is out of bounds for leaves.len() of {}, left leaf: \
                             {left:?}, right leaf: {right:?}",
                            leaf.ptr,
                            leaves.len()
                        );
                    }
                }

                nodes[i as usize] = Cell::new(parent_node);
            }

            current_level_start /= 2;
        }

        debug_assert!(root_set);

        leaves.push(Leaf::new(data.len() as u32));

        Self {
            nodes,
            data,
            leaves,
        }
    }
}

impl<T, A: Allocator> Bvh<Vec<T, A>, A> {
    pub fn elements(&self) -> &[T] {
        &self.data
    }
}

impl<L, A: Allocator> Bvh<L, A> {
    /// # Safety
    /// `idx` must be within `self.nodes`. Checked under `debug_assertions`.
    #[allow(clippy::missing_panics_doc)]
    pub unsafe fn get_node(&self, idx: u32) -> Node {
        debug_assert_lt!(u64::from(idx), u64::try_from(self.nodes.len()).unwrap());
        // SAFETY: the caller guarantees `idx` is in bounds.
        let ptr = unsafe { self.nodes.get_unchecked(idx as usize) };
        ptr.get()
    }
}

#[must_use]
pub const fn child_left(idx: u32) -> u32 {
    idx * 2
}

#[must_use]
pub const fn parent(idx: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(idx / 2)
}

#[must_use]
pub const fn child_right(idx: u32) -> u32 {
    idx * 2 + 1
}

#[must_use]
pub const fn sibling_right(idx: u32) -> Option<NonZeroU32> {
    //      1
    //    2   3
    //   45   67

    let tentative_next = idx + 1;

    if tentative_next.is_power_of_two() {
        // there is nothing to the right, we would be going to the next row
        None
    } else {
        Some(unsafe { NonZeroU32::new_unchecked(tentative_next) })
    }
}

pub const ROOT_IDX: u32 = 1;

fn process_input<I, T, A>(
    input: &[I],
    alloc: A,
    context: I::Context<'_>,
) -> (Vec<T, A>, Vec<Leaf, A>, Vec<glam::I16Vec2, A>)
where
    I: PointWithData<Unit = T>,
    T: Copy + 'static,
    A: Allocator + Clone,
{
    let mut result_data = Vec::new_in(alloc.clone());
    let mut indices = Vec::new_in(alloc.clone());
    let mut points = Vec::new_in(alloc);
    let mut current_point = None;

    for elem in input {
        let point = elem.point();

        if Some(point) != current_point {
            let index = unsafe { u32::try_from(result_data.len()).unwrap_unchecked() };
            indices.push(Leaf::new(index));
            points.push(point);
        }

        let data = elem.data(context);
        result_data.extend_from_slice(data);
        current_point = Some(point);
    }

    (result_data, indices, points)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use glam::I16Vec2;

    use crate::{Data, Leaf, Point, process_input, sibling_right};

    #[test]
    fn test_sibling_right() {
        assert_eq!(sibling_right(1), None);

        assert_eq!(sibling_right(2), NonZeroU32::new(3));
        assert_eq!(sibling_right(3), None);

        assert_eq!(sibling_right(4), NonZeroU32::new(5));
        assert_eq!(sibling_right(5), NonZeroU32::new(6));
        assert_eq!(sibling_right(6), NonZeroU32::new(7));
        assert_eq!(sibling_right(7), None);
    }

    #[derive(Clone)]
    struct TestPoint {
        point: I16Vec2,
        data: Vec<u8>,
    }

    impl Point for TestPoint {
        fn point(&self) -> I16Vec2 {
            self.point
        }
    }

    impl Data for TestPoint {
        type Unit = u8;

        fn data<'a: 'c, 'b: 'c, 'c>(&'a self, _context: Self::Context<'b>) -> &'c [Self::Unit] {
            &self.data
        }
    }

    #[test]
    fn test_process_input() {
        let input = vec![
            TestPoint {
                point: I16Vec2::new(0, 0),
                data: vec![1, 2],
            },
            TestPoint {
                point: I16Vec2::new(0, 0),
                data: vec![3, 4],
            },
            TestPoint {
                point: I16Vec2::new(1, 1),
                data: vec![5, 6],
            },
            TestPoint {
                point: I16Vec2::new(2, 2),
                data: vec![7, 8],
            },
            TestPoint {
                point: I16Vec2::new(2, 2),
                data: vec![9, 10],
            },
        ];

        let (result_data, indices, points) = process_input(&input, std::alloc::Global, ());

        assert_eq!(result_data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(indices, vec![Leaf::new(0), Leaf::new(4), Leaf::new(6)]);
        assert_eq!(points, vec![
            I16Vec2::new(0, 0),
            I16Vec2::new(1, 1),
            I16Vec2::new(2, 2),
        ]);
    }

    #[test]
    fn test_process_input_empty() {
        let input: Vec<TestPoint> = vec![];
        let (result_data, indices, points) = process_input(&input, std::alloc::Global, ());

        assert!(result_data.is_empty());
        assert!(indices.is_empty());
        assert!(points.is_empty());
    }

    #[test]
    fn test_process_input_single_point() {
        let input = vec![TestPoint {
            point: I16Vec2::new(5, 5),
            data: vec![42, 43],
        }];

        let (result_data, indices, points) = process_input(&input, std::alloc::Global, ());

        assert_eq!(result_data, vec![42, 43]);
        assert_eq!(indices, vec![Leaf::new(0)]);
        assert_eq!(points, vec![I16Vec2::new(5, 5)]);
    }

    #[test]
    fn test_process_input_all_unique_points() {
        let input = vec![
            TestPoint {
                point: I16Vec2::new(0, 0),
                data: vec![1],
            },
            TestPoint {
                point: I16Vec2::new(1, 1),
                data: vec![2],
            },
            TestPoint {
                point: I16Vec2::new(2, 2),
                data: vec![3],
            },
        ];

        let (result_data, indices, points) = process_input(&input, std::alloc::Global, ());

        assert_eq!(result_data, vec![1, 2, 3]);
        assert_eq!(indices, vec![Leaf::new(0), Leaf::new(1), Leaf::new(2)]);
        assert_eq!(points, vec![
            I16Vec2::new(0, 0),
            I16Vec2::new(1, 1),
            I16Vec2::new(2, 2)
        ]);
    }

    #[test]
    fn test_process_input_multiple_same_key() {
        let input = vec![
            TestPoint {
                point: I16Vec2::new(0, 0),
                data: vec![1],
            },
            TestPoint {
                point: I16Vec2::new(0, 0),
                data: vec![2],
            },
            TestPoint {
                point: I16Vec2::new(1, 1),
                data: vec![3],
            },
        ];

        let (result_data, indices, points) = process_input(&input, std::alloc::Global, ());

        assert_eq!(result_data, vec![1, 2, 3]);
        assert_eq!(indices, vec![Leaf::new(0), Leaf::new(2)]);
        assert_eq!(points, vec![I16Vec2::new(0, 0), I16Vec2::new(1, 1)]);
    }
}
