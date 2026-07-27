// todo: should we pack?
#[derive(Copy, Clone, Debug)]
pub struct Dfs {
    pub idx: u32,
    pub distance_to_leaf: u8,
}

impl Dfs {
    pub const fn new(depth: u8) -> Self {
        Self {
            idx: 0,
            distance_to_leaf: depth,
        }
    }

    pub fn left(self) -> Self {
        debug_assert!(
            self.distance_to_leaf != 0,
            "trying to go left on a leaf node"
        );
        Self {
            idx: self.idx + 1,
            distance_to_leaf: self.distance_to_leaf - 1,
        }
    }

    #[allow(unused)]
    pub fn full_left(self) -> Self {
        debug_assert!(
            self.distance_to_leaf != 0,
            "trying to go left on a leaf node"
        );

        Self {
            idx: self.idx + u32::from(self.distance_to_leaf),
            distance_to_leaf: 0,
        }
    }

    #[allow(unused)]
    pub fn full_right(self) -> Self {
        debug_assert!(
            self.distance_to_leaf != 0,
            "trying to go right on a leaf node"
        );

        Self {
            idx: self.idx + 2_u32.pow(u32::from(self.distance_to_leaf) + 1) - 2,
            distance_to_leaf: 0,
        }
    }

    pub fn right(self) -> Self {
        debug_assert!(
            self.distance_to_leaf != 0,
            "trying to go right on a leaf node"
        );
        Self {
            idx: self.idx + 2_u32.pow(u32::from(self.distance_to_leaf)),
            distance_to_leaf: self.distance_to_leaf - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_0() {
        /*
         * Tree (depth 0):
         *   0
         */
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 0,
        };

        assert_eq!(context.idx, 0);
        assert_eq!(context.distance_to_leaf, 0);
    }

    #[test]
    fn test_depth_1() {
        /*
         * Tree (depth 1):
         *     0
         *    / \
         *   1   2
         */
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 1,
        };

        let left = context.left();
        assert_eq!(left.idx, 1);
        assert_eq!(left.distance_to_leaf, 0);

        let right = context.right();
        assert_eq!(right.idx, 2);
        assert_eq!(right.distance_to_leaf, 0);
    }

    #[test]
    fn test_depth_2() {
        /*
         * Tree (depth 2):
         *        0
         *      /   \
         *     1     4
         *    / \   / \
         *   2   3 5   6
         */
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 2,
        };

        let left = context.left();
        assert_eq!(left.idx, 1);
        assert_eq!(left.distance_to_leaf, 1);

        let left_left = left.left();
        assert_eq!(left_left.idx, 2);
        assert_eq!(left_left.distance_to_leaf, 0);

        let left_right = left.right();
        assert_eq!(left_right.idx, 3);
        assert_eq!(left_right.distance_to_leaf, 0);

        let right = context.right();
        assert_eq!(right.idx, 4);
        assert_eq!(right.distance_to_leaf, 1);

        let right_left = right.left();
        assert_eq!(right_left.idx, 5);
        assert_eq!(right_left.distance_to_leaf, 0);

        let right_right = right.right();
        assert_eq!(right_right.idx, 6);
        assert_eq!(right_right.distance_to_leaf, 0);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_depth_3() {
        /*
         * Tree (depth 3):
         *            0
         *         /     \
         *        1       8
         *      /  \    /   \
         *     2    5  9    12
         *    / \  / \ / \  / \
         *   3  4 6  7 10 11 13 14
         */
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 3,
        };

        let left = context.left();
        assert_eq!(left.idx, 1);
        assert_eq!(left.distance_to_leaf, 2);

        let left_left = left.left();
        assert_eq!(left_left.idx, 2);
        assert_eq!(left_left.distance_to_leaf, 1);

        let left_left_left = left_left.left();
        assert_eq!(left_left_left.idx, 3);
        assert_eq!(left_left_left.distance_to_leaf, 0);

        let left_left_right = left_left.right();
        assert_eq!(left_left_right.idx, 4);
        assert_eq!(left_left_right.distance_to_leaf, 0);

        let left_right = left.right();
        assert_eq!(left_right.idx, 5);
        assert_eq!(left_right.distance_to_leaf, 1);

        let left_right_left = left_right.left();
        assert_eq!(left_right_left.idx, 6);
        assert_eq!(left_right_left.distance_to_leaf, 0);

        let left_right_right = left_right.right();
        assert_eq!(left_right_right.idx, 7);
        assert_eq!(left_right_right.distance_to_leaf, 0);

        let right = context.right();
        assert_eq!(right.idx, 8);
        assert_eq!(right.distance_to_leaf, 2);

        let right_left = right.left();
        assert_eq!(right_left.idx, 9);
        assert_eq!(right_left.distance_to_leaf, 1);

        let right_left_left = right_left.left();
        assert_eq!(right_left_left.idx, 10);
        assert_eq!(right_left_left.distance_to_leaf, 0);

        let right_left_right = right_left.right();
        assert_eq!(right_left_right.idx, 11);
        assert_eq!(right_left_right.distance_to_leaf, 0);

        let right_right = right.right();
        assert_eq!(right_right.idx, 12);
        assert_eq!(right_right.distance_to_leaf, 1);

        let right_right_left = right_right.left();
        assert_eq!(right_right_left.idx, 13);
        assert_eq!(right_right_left.distance_to_leaf, 0);

        let right_right_right = right_right.right();
        assert_eq!(right_right_right.idx, 14);
        assert_eq!(right_right_right.distance_to_leaf, 0);
    }

    #[test]
    fn test_full_left() {
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 3,
        };

        let full_left = context.full_left();
        assert_eq!(full_left.idx, 3);
        assert_eq!(full_left.distance_to_leaf, 0);

        let recursive_left = context.left().left().left();
        assert_eq!(recursive_left.idx, 3);
        assert_eq!(recursive_left.distance_to_leaf, 0);
    }

    #[test]
    fn test_full_right() {
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 3,
        };

        let full_right = context.full_right();
        assert_eq!(full_right.idx, 14);
        assert_eq!(full_right.distance_to_leaf, 0);

        let recursive_right = context.right().right().right();
        assert_eq!(recursive_right.idx, 14);
        assert_eq!(recursive_right.distance_to_leaf, 0);
    }

    #[test]
    #[should_panic(expected = "trying to go right on a leaf node")]
    fn test_full_right_depth_0() {
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 0,
        };

        // Full right on a leaf node should panic
        context.full_right();
    }

    #[test]
    fn test_full_right_depth_1() {
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 1,
        };

        let full_right = context.full_right();
        assert_eq!(full_right.idx, 2);
        assert_eq!(full_right.distance_to_leaf, 0);

        let recursive_right = context.right();
        assert_eq!(recursive_right.idx, 2);
        assert_eq!(recursive_right.distance_to_leaf, 0);
    }

    #[test]
    fn test_full_right_depth_2() {
        let context = Dfs {
            idx: 0,
            distance_to_leaf: 2,
        };

        let full_right = context.full_right();
        assert_eq!(full_right.idx, 6);
        assert_eq!(full_right.distance_to_leaf, 0);

        let recursive_right = context.right().right();
        assert_eq!(recursive_right.idx, 6);
        assert_eq!(recursive_right.distance_to_leaf, 0);
    }
}
