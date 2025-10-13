use wide::{CmpGt, CmpLt, f64x4};

#[derive(Debug, Clone)]
pub struct ParallelStats {
    counts: Vec<u64>,
    means: Vec<f64>,
    m2s: Vec<f64>,
    mins: Vec<f64>,
    maxs: Vec<f64>,
    width: usize, // Number of parallel statistics being tracked
}

impl ParallelStats {
    #[must_use]
    pub fn new(width: usize) -> Self {
        Self {
            counts: vec![0; width],
            means: vec![0.0; width],
            m2s: vec![0.0; width],
            mins: vec![f64::INFINITY; width],
            maxs: vec![f64::NEG_INFINITY; width],
            width,
        }
    }

    /// Update multiple parallel running statistics with SIMD when possible
    /// Each slice must be the same length as width
    pub fn update(&mut self, values: &[f64]) {
        assert_eq!(values.len(), self.width, "Input length must match width");

        let mut idx = 0;

        // Process in SIMD chunks of 4 parallel stats where possible
        while idx + 4 <= self.width {
            self.simd_update(idx, &values[idx..idx + 4]);
            idx += 4;
        }

        // Handle remaining elements with scalar operations
        while idx < self.width {
            self.update_single(idx, values[idx]);
            idx += 1;
        }
    }

    fn simd_update(&mut self, chunk_start: usize, values: &[f64]) {
        if values.len() != 4 {
            // Fallback to scalar for incomplete chunks
            for (i, &value) in values.iter().enumerate() {
                self.update_single(chunk_start + i, value);
            }
            return;
        }

        let chunk_end = chunk_start + 4;
        let values_simd = f64x4::new([values[0], values[1], values[2], values[3]]);
        let means_simd = f64x4::new([
            self.means[chunk_start],
            self.means[chunk_start + 1],
            self.means[chunk_start + 2],
            self.means[chunk_start + 3],
        ]);
        let m2s_simd = f64x4::new([
            self.m2s[chunk_start],
            self.m2s[chunk_start + 1],
            self.m2s[chunk_start + 2],
            self.m2s[chunk_start + 3],
        ]);
        let mins_simd = f64x4::new([
            self.mins[chunk_start],
            self.mins[chunk_start + 1],
            self.mins[chunk_start + 2],
            self.mins[chunk_start + 3],
        ]);
        let maxs_simd = f64x4::new([
            self.maxs[chunk_start],
            self.maxs[chunk_start + 1],
            self.maxs[chunk_start + 2],
            self.maxs[chunk_start + 3],
        ]);
        let counts_chunk = &mut self.counts[chunk_start..chunk_end];

        // Update counts
        for count in counts_chunk.iter_mut() {
            *count += 1;
        }

        // Convert counts to f64x4 for SIMD division
        let counts_f64 = f64x4::new([
            counts_chunk[0] as f64,
            counts_chunk[1] as f64,
            counts_chunk[2] as f64,
            counts_chunk[3] as f64,
        ]);

        // Update means (Welford's algorithm)
        let delta = values_simd - means_simd;
        let new_means = means_simd + delta / counts_f64;

        // Update M2
        let delta2 = values_simd - new_means;
        let new_m2s = m2s_simd + delta * delta2;

        // Update mins and maxs using SIMD comparisons
        let values_lt_mins = values_simd.cmp_lt(mins_simd);
        let values_gt_maxs = values_simd.cmp_gt(maxs_simd);

        // Blend the values based on comparisons
        let new_mins = values_lt_mins.blend(values_simd, mins_simd);
        let new_maxs = values_gt_maxs.blend(values_simd, maxs_simd);

        // Store results back
        let new_means_array = new_means.to_array();
        let new_m2s_array = new_m2s.to_array();
        let new_mins_array = new_mins.to_array();
        let new_maxs_array = new_maxs.to_array();

        self.means[chunk_start..chunk_start + 4].copy_from_slice(&new_means_array);
        self.m2s[chunk_start..chunk_start + 4].copy_from_slice(&new_m2s_array);
        self.mins[chunk_start..chunk_start + 4].copy_from_slice(&new_mins_array);
        self.maxs[chunk_start..chunk_start + 4].copy_from_slice(&new_maxs_array);
    }

    fn update_single(&mut self, idx: usize, value: f64) {
        self.counts[idx] += 1;
        let count = self.counts[idx] as f64;

        // Update min/max
        self.mins[idx] = self.mins[idx].min(value);
        self.maxs[idx] = self.maxs[idx].max(value);

        // Update mean and M2 using Welford's algorithm
        let delta = value - self.means[idx];
        self.means[idx] += delta / count;
        let delta2 = value - self.means[idx];
        self.m2s[idx] += delta * delta2;
    }

    #[must_use]
    pub fn count(&self, idx: usize) -> u64 {
        self.counts[idx]
    }

    #[must_use]
    pub fn mean(&self, idx: usize) -> Option<f64> {
        if self.counts[idx] > 0 {
            Some(self.means[idx])
        } else {
            None
        }
    }

    #[must_use]
    pub fn variance(&self, idx: usize) -> Option<f64> {
        if self.counts[idx] > 1 {
            Some(self.m2s[idx] / (self.counts[idx] - 1) as f64)
        } else {
            None
        }
    }

    #[must_use]
    pub fn std_dev(&self, idx: usize) -> Option<f64> {
        self.variance(idx).map(f64::sqrt)
    }

    #[must_use]
    pub fn min(&self, idx: usize) -> Option<f64> {
        if self.counts[idx] > 0 {
            Some(self.mins[idx])
        } else {
            None
        }
    }

    #[must_use]
    pub fn max(&self, idx: usize) -> Option<f64> {
        if self.counts[idx] > 0 {
            Some(self.maxs[idx])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    #[test]
    fn test_parallel_stats() {
        let mut stats = ParallelStats::new(4);

        // Update with 4 independent series
        let updates = [
            // Series 1: [2.0, 4.0]
            // Series 2: [3.0, 6.0]
            // Series 3: [4.0, 8.0]
            // Series 4: [5.0, 10.0]
            vec![2.0, 3.0, 4.0, 5.0],
            vec![4.0, 6.0, 8.0, 10.0],
        ];

        for update in &updates {
            stats.update(update);
        }

        // Test series 1 (index 0)
        assert_eq!(stats.count(0), 2);
        assert_relative_eq!(stats.mean(0).unwrap(), 3.0);
        assert_relative_eq!(stats.min(0).unwrap(), 2.0);
        assert_relative_eq!(stats.max(0).unwrap(), 4.0);

        // Test series 2 (index 1)
        assert_eq!(stats.count(1), 2);
        assert_relative_eq!(stats.mean(1).unwrap(), 4.5);
        assert_relative_eq!(stats.min(1).unwrap(), 3.0);
        assert_relative_eq!(stats.max(1).unwrap(), 6.0);

        // Test series 3 (index 2)
        assert_eq!(stats.count(2), 2);
        assert_relative_eq!(stats.mean(2).unwrap(), 6.0);
        assert_relative_eq!(stats.min(2).unwrap(), 4.0);
        assert_relative_eq!(stats.max(2).unwrap(), 8.0);

        // Test series 4 (index 3)
        assert_eq!(stats.count(3), 2);
        assert_relative_eq!(stats.mean(3).unwrap(), 7.5);
        assert_relative_eq!(stats.min(3).unwrap(), 5.0);
        assert_relative_eq!(stats.max(3).unwrap(), 10.0);
    }

    #[test]
    fn test_non_simd_width() {
        // Test with width that's not a multiple of 4
        let mut stats = ParallelStats::new(5);
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        stats.update(&values);

        for i in 0..5 {
            assert_eq!(stats.count(i), 1);
            assert_relative_eq!(stats.mean(i).unwrap(), (i + 1) as f64);
        }
    }
}
