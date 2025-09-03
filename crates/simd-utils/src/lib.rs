#![cfg_attr(feature = "nightly", feature(portable_simd))]
#![cfg_attr(feature = "nightly", feature(trusted_len))]
#![cfg_attr(feature = "nightly", feature(pointer_is_aligned_to))]

#[cfg(feature = "nightly")]
use core::simd;
#[cfg(not(feature = "nightly"))]
use std::iter::zip;
#[cfg(feature = "nightly")]
use std::{
    iter::zip,
    simd::{LaneCount, Mask, MaskElement, Simd, SupportedLaneCount, cmp::SimdPartialEq},
};

#[cfg(feature = "nightly")]
use crate::one_bit_positions::OneBitPositionsExt;

#[cfg(feature = "nightly")]
mod one_bit_positions;

/// Efficiently compares two slices and copies `current` into `prev`, calling `on_diff` for each difference found.
///
/// This function uses SIMD instructions when possible to accelerate the comparison and copy operations.
/// It handles two main cases:
///
/// 1. When `prev` and `current` have the same alignment offset:
///    - Uses aligned SIMD loads/stores for better performance
///    - Processes data in three parts:
///      a. Scalar comparison of unaligned prefix
///      b. SIMD comparison of aligned middle section
///      c. Scalar comparison of unaligned suffix
///
/// 2. When `prev` and `current` have different alignment offsets:
///    - Uses unaligned SIMD loads/stores
///    - Processes data in chunks:
///      a. SIMD comparison of main chunks with unaligned access
///      b. Scalar comparison of remaining elements
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
/// - Type `T` must support SIMD operations and comparisons
/// - SIMD alignment must not exceed 64 bytes
#[cfg(feature = "nightly")]
pub fn copy_and_get_diff<T, const LANES: usize>(
    prev: &mut [T],
    current: &[T],
    mut on_diff: impl FnMut(usize, &T, &T),
) where
    Simd<T, LANES>: AsMut<[T; LANES]> + SimdPartialEq,
    T: simd::SimdElement + PartialEq + std::fmt::Debug,
    <T as simd::SimdElement>::Mask: MaskElement,
    LaneCount<LANES>: SupportedLaneCount,
    <Simd<T, LANES> as SimdPartialEq>::Mask: Into<Mask<<T as simd::SimdElement>::Mask, LANES>>,
{
    // Verify SIMD alignment requirement at compile time
    const {
        assert!(
            align_of::<Simd<T, LANES>>() <= 64,
            "alignment of Simd<T, LANES> must be <= 64 bytes"
        );
    }

    assert_eq!(
        prev.len(),
        current.len(),
        "prev and current must have the same length"
    );

    // Split slices into SIMD-aligned sections
    let (before_prev, prev_simd, after_prev) = prev.as_simd_mut::<LANES>();
    let (before_current, current_simd, after_current) = current.as_simd::<LANES>();

    if before_prev.len() == before_current.len() {
        // CASE 1: Same alignment offset - can use aligned SIMD operations
        debug_assert_eq!(
            prev_simd.len(),
            current_simd.len(),
            "prev_simd and current_simd must have the same length"
        );
        debug_assert_eq!(
            after_prev.len(),
            after_current.len(),
            "after_prev and after_current must have the same length"
        );

        // Handle unaligned prefix
        copy_and_get_diff_scalar(0, before_prev, before_current, &mut on_diff);

        // Process aligned middle section using SIMD
        let mut idx = before_prev.len();
        for (prev, current) in zip(prev_simd, current_simd) {
            let not_equal = prev.simd_ne(*current);
            let not_equal = not_equal.into();
            let bitmask = Mask::to_bitmask(not_equal);

            // Process each difference found in the SIMD lane
            for local_idx in bitmask.one_positions() {
                let prev = prev[local_idx];
                let current = current[local_idx];
                debug_assert_ne!(prev, current);
                on_diff(idx + local_idx, &prev, &current);
            }

            idx += LANES;
            current.copy_to_slice(prev.as_mut());
        }

        // Handle unaligned suffix
        copy_and_get_diff_scalar(idx, after_prev, after_current, &mut on_diff);
    } else {
        // CASE 2: Different alignment offsets - use unaligned SIMD operations
        let (prev_chunks, prev_remaining) = prev.as_chunks_mut::<LANES>();
        let (current_chunks, current_remaining) = current.as_chunks::<LANES>();

        debug_assert_eq!(
            prev_chunks.len(),
            current_chunks.len(),
            "prev_chunks and current_chunks must have the same length"
        );
        debug_assert_eq!(
            prev_remaining.len(),
            current_remaining.len(),
            "prev_remaining and current_remaining must have the same length"
        );

        let mut idx = 0;
        // Process main chunks with unaligned SIMD operations
        for (prev, current) in zip(prev_chunks, current_chunks) {
            let prev_simd = Simd::from_array(*prev);
            let current_simd = Simd::from_array(*current);

            let not_equal = prev_simd.simd_ne(current_simd);
            let not_equal = not_equal.into();
            let bitmask = Mask::to_bitmask(not_equal);

            for local_idx in bitmask.one_positions() {
                let prev = prev[local_idx];
                let current = current[local_idx];
                debug_assert_ne!(prev, current);
                on_diff(idx + local_idx, &prev, &current);
            }

            idx += LANES;
            current_simd.copy_to_slice(prev);
        }

        // Handle remaining elements
        copy_and_get_diff_scalar(idx, prev_remaining, current_remaining, &mut on_diff);
    }
}

/// Fallback implementation for stable Rust (non-SIMD)
#[cfg(not(feature = "nightly"))]
pub fn copy_and_get_diff<T, const LANES: usize>(
    prev: &mut [T],
    current: &[T],
    on_diff: impl FnMut(usize, &T, &T),
) where
    T: Copy + PartialEq + std::fmt::Debug,
{
    copy_and_get_diff_scalar(0, prev, current, on_diff);
}

/// Scalar (non-SIMD) implementation of [`copy_and_get_diff`] for handling small sections
/// or remainders that can't be processed with SIMD.
fn copy_and_get_diff_scalar<T>(
    start_idx: usize,
    prev: &mut [T],
    current: &[T],
    mut on_diff: impl FnMut(usize, &T, &T),
) where
    T: Copy + PartialEq + std::fmt::Debug,
{
    let mut idx = start_idx;
    debug_assert_eq!(prev.len(), current.len());

    for (prev, current) in zip(prev, current) {
        if prev != current {
            debug_assert_ne!(prev, current);
            on_diff(idx, prev, current);
        }
        *prev = *current;
        idx += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use super::*;

    const LANES: usize = 8;

    #[cfg(feature = "nightly")]
    const SIMD_U32_ALIGN: usize = std::mem::align_of::<Simd<u32, LANES>>();

    #[cfg(feature = "nightly")]
    use aligned_vec::{AVec, RuntimeAlign};
    #[cfg(feature = "nightly")]
    use proptest::prelude::*;

    // Helper function to collect differences (SIMD version)
    #[cfg(feature = "nightly")]
    fn collect_diffs<T>(prev_raw: &[T], current_raw: &[T]) -> Vec<(usize, T, T)>
    where
        Simd<T, LANES>: AsMut<[T; LANES]> + SimdPartialEq,
        T: simd::SimdElement + PartialEq + Debug,
        <T as simd::SimdElement>::Mask: MaskElement,
        LaneCount<LANES>: SupportedLaneCount,
        <Simd<T, LANES> as SimdPartialEq>::Mask: Into<Mask<<T as simd::SimdElement>::Mask, LANES>>,
    {
        // convert prev and current to simd-aligned arrays
        let mut prev: AVec<T, RuntimeAlign> = AVec::from_iter(64, prev_raw.iter().copied());
        let current: AVec<T, RuntimeAlign> = AVec::from_iter(64, current_raw.iter().copied());

        let mut diffs = Vec::new();
        copy_and_get_diff::<_, LANES>(&mut prev, &current, |idx, prev, curr| {
            diffs.push((idx, *prev, *curr));
        });
        diffs
    }

    // Helper function to collect differences (non-SIMD version)
    #[cfg(not(feature = "nightly"))]
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

    // Generate arrays of various sizes to test SIMD boundary conditions
    #[cfg(feature = "nightly")]
    fn generate_array_strategy<T>(min_size: usize) -> impl Strategy<Value = Vec<T>>
    where
        T: simd::SimdElement + Arbitrary + 'static,
    {
        prop::collection::vec(any::<T>(), min_size..=min_size + LANES * 2)
    }

    // Generate arrays of an exact size
    #[cfg(feature = "nightly")]
    fn generate_exact_array_strategy<T>(size: usize) -> impl Strategy<Value = Vec<T>>
    where
        T: simd::SimdElement + Arbitrary + 'static,
    {
        prop::collection::vec(any::<T>(), size)
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

    // Simple non-SIMD test for stable Rust
    #[cfg(not(feature = "nightly"))]
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

    #[cfg(feature = "nightly")]
    proptest! {
        // Test with u32 arrays of various sizes
        #[test]
        fn test_u32_arrays(
            current in generate_array_strategy::<u32>(LANES * 2)
        ) {
            let mut prev = current.clone();
            // Modify some elements to create differences
            if !prev.is_empty() {
                let prev_len = prev.len();
                prev[prev_len / 2] = prev[prev_len / 2].wrapping_add(1);
                if prev_len > 1 {
                    prev[0] = prev[0].wrapping_add(1);
                }
            }

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }

        // Test with i32 arrays including negative numbers
        #[test]
        fn test_i32_arrays(
            current in generate_array_strategy::<i32>(LANES * 2)
        ) {
            let mut prev = current.clone();
            if !prev.is_empty() {
                let prev_len = prev.len();
                prev[prev_len / 2] = prev[prev_len / 2].wrapping_add(1);
                if prev_len > 1 {
                    prev[0] = prev[0].wrapping_sub(1);
                }
            }

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }

        // Test with varying align offset
        #[test]
        fn test_varying_align_offset(
            current in generate_exact_array_strategy::<u32>(LANES * 4),
            mut prev in generate_exact_array_strategy::<u32>(LANES * 4)
        ) {
            // Ensure that [`current`] and [`prev`] have a different align offset
            let current = &current[..(current.len() - 1)];
            let mut prev = prev.as_mut_slice();

            if prev.as_ptr().align_offset(SIMD_U32_ALIGN) == current.as_ptr().align_offset(SIMD_U32_ALIGN) {
                // Offset [`prev`] by 1 element to get a different align offset
                prev = &mut prev[1..];
            } else {
                // Keep the align offset of [`prev`] the same but truncate it to the same size as
                // [`current`]
                let len = prev.len() - 1;
                prev = &mut prev[..len];
            }

            assert_eq!(prev.len(), current.len());
            assert_ne!(prev.as_ptr().align_offset(SIMD_U32_ALIGN), current.as_ptr().align_offset(SIMD_U32_ALIGN));

            let diffs = collect_diffs(prev, current);
            verify_differences(prev, current, &diffs);
        }

        // Test with same align offset but not aligned with a simd vector
        #[test]
        fn test_same_align_offset(
            mut data in generate_exact_array_strategy::<u32>(LANES * 4 + 1),
        ) {
            let mut data = data.as_mut_slice();
            if data.as_ptr().is_aligned_to(SIMD_U32_ALIGN) {
                data = &mut data[1..];
            }

            let len = LANES * 2;
            let (prev, current) = data.split_at_mut(len);
            let current = &current[..len];

            assert_eq!(prev.len(), current.len());
            assert!(!prev.as_ptr().is_aligned_to(SIMD_U32_ALIGN));
            assert!(!current.as_ptr().is_aligned_to(SIMD_U32_ALIGN));
            assert_eq!(prev.as_ptr().align_offset(SIMD_U32_ALIGN), current.as_ptr().align_offset(SIMD_U32_ALIGN));

            let diffs = collect_diffs(prev, current);
            verify_differences(prev, current, &diffs);
        }

        // Test with exact SIMD lane size
        #[test]
        fn test_exact_lane_size(
            current in generate_array_strategy::<u32>(LANES)
        ) {
            let mut prev = current.clone();
            if !prev.is_empty() {
                prev[0] = prev[0].wrapping_add(1);
            }

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }

        // Test with arrays smaller than SIMD lane size
        #[test]
        fn test_small_arrays(
            current in generate_array_strategy::<u32>(LANES / 2)
        ) {
            let mut prev = current.clone();
            if !prev.is_empty() {
                prev[0] = prev[0].wrapping_add(1);
            }

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }

        // Test with no differences
        #[test]
        fn test_no_differences(
            current in generate_array_strategy::<u32>(LANES * 2)
        ) {
            let diffs = collect_diffs(&current, &current);
            assert!(diffs.is_empty(), "Expected no differences");
        }

        // Test with all elements different
        #[test]
        fn test_all_different(
            current in generate_array_strategy::<u32>(LANES * 2)
        ) {
            let prev = current.iter()
                .map(|x| x.wrapping_add(1))
                .collect::<Vec<_>>();

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }

        // Test edge case with alternating differences
        #[test]
        fn test_alternating_differences(
            current in generate_array_strategy::<u32>(LANES * 2)
        ) {
            let mut prev = current.clone();
            for i in (0..prev.len()).step_by(2) {
                prev[i] = prev[i].wrapping_add(1);
            }

            let diffs = collect_diffs(&prev, &current);
            verify_differences(&prev, &current, &diffs);
        }
    }
}
