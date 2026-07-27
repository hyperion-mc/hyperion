use std::{
    alloc::Allocator,
    collections::VecDeque,
    fmt::{Debug, Write as _},
};

use crate::{Aabb, Bvh, ROOT_IDX, child_left, child_right, node::Expanded};

struct Element {
    idx: u32,
    depth: usize,
}

impl<T: Debug, A: Allocator> Bvh<Vec<T, A>, A> {
    pub fn print(&self) -> String {
        let mut output = String::new();
        self.print_helper(&mut output);

        // trim last newline
        if output.ends_with('\n') {
            output.pop();
        }

        output
    }

    fn print_helper(&self, output: &mut String) {
        let mut queue = VecDeque::new();
        queue.push_back(Element {
            idx: ROOT_IDX,
            depth: 0,
        });

        while let Some(Element { idx, depth }) = queue.pop_back() {
            let indent = "  ".repeat(depth);
            let node = unsafe { self.get_node(idx) };

            // println!("idx {idx}, node {node:?}");

            match node.into_expanded() {
                Some(Expanded::Aabb(aabb)) => {
                    if aabb == Aabb::INVALID {
                        continue;
                    }

                    let _unused = writeln!(output, "{idx:02}\t{indent}Internal({aabb:?})");

                    let left = child_left(idx);
                    let right = child_right(idx);

                    queue.push_back(Element {
                        idx: left,
                        depth: depth + 1,
                    });

                    queue.push_back(Element {
                        idx: right,
                        depth: depth + 1,
                    });
                }
                Some(Expanded::Leaf(leaf)) => {
                    let leaf_point = leaf.point;

                    let Some(left_leaf) = self.leaves.get(leaf.ptr as usize) else {
                        unreachable!(
                            "leaf.ptr {} is out of bounds for leaf at point {leaf_point} at index \
                             {idx}",
                            leaf.ptr
                        );
                    };

                    let element_idx_start = left_leaf.element_index;

                    let next_ptr = leaf.ptr + 1;
                    let element_idx_end = self.leaves[next_ptr as usize].element_index;

                    let data = &self.data[element_idx_start as usize..element_idx_end as usize];

                    let _unused =
                        writeln!(output, "{idx:02}\t{indent}Leaf({leaf_point} => {data:?})");
                }
                None => {}
            }
        }
    }
}
