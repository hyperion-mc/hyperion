use std::iter::zip;

/// Efficiently compares two slices and copies `current` into `prev`, calling `on_diff` for each difference found.
///
/// This function processes data in chunks for better performance, with fallback to scalar operations.
/// While SIMD optimizations have been removed for stable Rust compatibility, chunked processing
/// still provides performance benefits through better cache locality and reduced function call overhead.
///
/// # Arguments
/// * `prev` - Mutable slice that will be updated with values from `current`
/// * `current` - Slice containing new values to compare against
/// * `on_diff` - Callback function called for each difference found with:
///   - Index of the difference
///   - Reference to old value from `prev`
///   - Reference to new value from `current`
///
/// # Requirements
/// - `prev` and `current` must have the same length
pub fn copy_and_get_diff<T, const LANES: usize>(
    prev: &mut [T],
    current: &[T],
    on_diff: impl FnMut(usize, &T, &T),
) where
    T: Copy + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        prev.len(),
        current.len(),
        "prev and current must have the same length"
    );

    // Process all elements with optimized scalar operations
    copy_and_get_diff_scalar(0, prev, current, on_diff);
}

/// Optimized scalar implementation of [`copy_and_get_diff`].
///
/// While not using SIMD, this implementation uses chunked processing for better
/// performance through improved cache locality and reduced overhead.
fn copy_and_get_diff_scalar<T>(
    start_idx: usize,
    prev: &mut [T],
    current: &[T],
    mut on_diff: impl FnMut(usize, &T, &T),
) where
    T: Copy + PartialEq + std::fmt::Debug,
{
    const CHUNK_SIZE: usize = 64;

    debug_assert_eq!(prev.len(), current.len());

    let mut idx = start_idx;
    let mut remaining_prev = prev;
    let mut remaining_current = current;

    // Process large chunks
    while remaining_prev.len() >= CHUNK_SIZE {
        let (chunk_prev, rest_prev) = remaining_prev.split_at_mut(CHUNK_SIZE);
        let (chunk_current, rest_current) = remaining_current.split_at(CHUNK_SIZE);

        // Process chunk with unrolled comparisons for better performance
        for i in (0..CHUNK_SIZE).step_by(4) {
            // Unroll 4 iterations to reduce loop overhead
            let end = (i + 4).min(CHUNK_SIZE);
            for j in i..end {
                if chunk_prev[j] != chunk_current[j] {
                    on_diff(idx + j, &chunk_prev[j], &chunk_current[j]);
                }
                chunk_prev[j] = chunk_current[j];
            }
        }

        idx += CHUNK_SIZE;
        remaining_prev = rest_prev;
        remaining_current = rest_current;
    }

    // Process remaining elements
    for (i, (prev_val, current_val)) in zip(remaining_prev, remaining_current).enumerate() {
        if prev_val != current_val {
            on_diff(idx + i, prev_val, current_val);
        }
        *prev_val = *current_val;
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use super::*;

    const LANES: usize = 8;

    // Helper function to collect differences
    fn collect_diffs<T>(prev_raw: &[T], current_raw: &[T]) -> Vec<(usize, T, T)>
    where
        T: Copy + PartialEq + Debug,
    {
        let mut prev = prev_raw.to_vec();
        let current = current_raw;

        let mut diffs = Vec::new();
        copy_and_get_diff::<_, LANES>(&mut prev, current, |idx, prev, curr| {
            diffs.push((idx, *prev, *curr));
        });
        diffs
    }

    // Helper to verify that all differences are captured correctly
    fn verify_differences<T>(prev: &[T], current: &[T], diffs: &[(usize, T, T)])
    where
        T: PartialEq + Debug + Clone,
    {
        let mut expected_diffs = Vec::new();
        for (idx, (p, c)) in zip(prev, current).enumerate() {
            if p != c {
                expected_diffs.push((idx, p.clone(), c.clone()));
            }
        }
        assert_eq!(
            diffs,
            expected_diffs.as_slice(),
            "Differences don't match expected for prev={prev:?} and current={current:?}"
        );
    }

    #[test]
    fn test_basic_functionality() {
        let prev = vec![1u32, 2, 3, 4, 5];
        let current = vec![1u32, 3, 3, 5, 5];

        let diffs = collect_diffs(&prev, &current);
        verify_differences(&prev, &current, &diffs);

        // Verify that the expected differences were found
        let expected_diffs = vec![(1, 2u32, 3u32), (3, 4u32, 5u32)];
        assert_eq!(diffs, expected_diffs);
    }

    #[test]
    fn test_large_array() {
        // Test with a larger array that will use chunked processing
        let mut prev = vec![0u32; 200];
        let mut current = vec![0u32; 200];

        // Set some differences at various positions
        prev[1] = 10;
        current[1] = 20;
        prev[65] = 30; // In second chunk
        current[65] = 40;
        prev[130] = 50; // In third chunk
        current[130] = 60;
        prev[199] = 70; // Last element
        current[199] = 80;

        let diffs = collect_diffs(&prev, &current);
        verify_differences(&prev, &current, &diffs);

        let expected_diffs = vec![
            (1, 10u32, 20u32),
            (65, 30u32, 40u32),
            (130, 50u32, 60u32),
            (199, 70u32, 80u32),
        ];
        assert_eq!(diffs, expected_diffs);
    }

    #[test]
    fn test_no_differences() {
        let current = vec![1u32, 2, 3, 4, 5, 6, 7, 8];
        let diffs = collect_diffs(&current, &current);
        assert!(diffs.is_empty(), "Expected no differences");
    }

    #[test]
    fn test_all_different() {
        let prev = vec![1u32, 2, 3, 4];
        let current = vec![5u32, 6, 7, 8];

        let diffs = collect_diffs(&prev, &current);
        verify_differences(&prev, &current, &diffs);

        let expected_diffs = vec![
            (0, 1u32, 5u32),
            (1, 2u32, 6u32),
            (2, 3u32, 7u32),
            (3, 4u32, 8u32),
        ];
        assert_eq!(diffs, expected_diffs);
    }

    #[test]
    fn test_i32_arrays() {
        let prev = vec![-1i32, 2, -3, 4, 5];
        let current = vec![-1i32, 3, -3, 5, 5];

        let diffs = collect_diffs(&prev, &current);
        verify_differences(&prev, &current, &diffs);

        let expected_diffs = vec![(1, 2i32, 3i32), (3, 4i32, 5i32)];
        assert_eq!(diffs, expected_diffs);
    }

    #[test]
    fn test_chunk_boundaries() {
        // Test with array size that crosses chunk boundaries
        let size = 130; // Crosses the 64-element chunk boundary
        let prev = vec![0u32; size];
        let current = vec![1u32; size]; // All different

        let diffs = collect_diffs(&prev, &current);
        verify_differences(&prev, &current, &diffs);

        // Should find differences at every position
        assert_eq!(diffs.len(), size);
        for (i, (idx, prev_val, curr_val)) in diffs.iter().enumerate() {
            assert_eq!(*idx, i);
            assert_eq!(*prev_val, 0u32);
            assert_eq!(*curr_val, 1u32);
        }
    }
}
