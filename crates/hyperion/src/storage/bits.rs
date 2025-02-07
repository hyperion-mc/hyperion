//! Utilities for working with bits.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "this is azalea code and I do not want to document it as of now."
)]

// from azalea
use std::{error::Error, fmt};

// this is from minecraft's code
// yeah idk either
const MAGIC: [(i32, i32, i32); 64] = [
    (-1, -1, 0),
    (-0x8000_0000, 0, 0),
    (1_431_655_765, 1_431_655_765, 0),
    (-0x8000_0000, 0, 1),
    (858_993_459, 858_993_459, 0),
    (715_827_882, 715_827_882, 0),
    (613_566_756, 613_566_756, 0),
    (-0x8000_0000, 0, 2),
    (477_218_588, 477_218_588, 0),
    (429_496_729, 429_496_729, 0),
    (390_451_572, 390_451_572, 0),
    (357_913_941, 357_913_941, 0),
    (330_382_099, 330_382_099, 0),
    (306_783_378, 306_783_378, 0),
    (286_331_153, 286_331_153, 0),
    (-0x8000_0000, 0, 3),
    (0x0F0F_0F0F, 0x0F0F_0F0F, 0),
    (238_609_294, 238_609_294, 0),
    (226_050_910, 226_050_910, 0),
    (214_748_364, 214_748_364, 0),
    (204_522_252, 204_522_252, 0),
    (195_225_786, 195_225_786, 0),
    (186_737_708, 186_737_708, 0),
    (178_956_970, 178_956_970, 0),
    (171_798_691, 171_798_691, 0),
    (165_191_049, 165_191_049, 0),
    (159_072_862, 159_072_862, 0),
    (153_391_689, 153_391_689, 0),
    (148_102_320, 148_102_320, 0),
    (143_165_576, 143_165_576, 0),
    (138_547_332, 138_547_332, 0),
    (-0x8000_0000, 0, 4),
    (130_150_524, 130_150_524, 0),
    (126_322_567, 126_322_567, 0),
    (122_713_351, 122_713_351, 0),
    (119_304_647, 119_304_647, 0),
    (116_080_197, 116_080_197, 0),
    (113_025_455, 113_025_455, 0),
    (110_127_366, 110_127_366, 0),
    (107_374_182, 107_374_182, 0),
    (104_755_299, 104_755_299, 0),
    (102_261_126, 102_261_126, 0),
    (99_882_960, 99_882_960, 0),
    (97_612_893, 97_612_893, 0),
    (95_443_717, 95_443_717, 0),
    (93_368_854, 93_368_854, 0),
    (91_382_282, 91_382_282, 0),
    (89_478_485, 89_478_485, 0),
    (87_652_393, 87_652_393, 0),
    (85_899_345, 85_899_345, 0),
    (84_215_045, 84_215_045, 0),
    (82_595_524, 82_595_524, 0),
    (81_037_118, 81_037_118, 0),
    (79_536_431, 79_536_431, 0),
    (78_090_314, 78_090_314, 0),
    (76_695_844, 76_695_844, 0),
    (75_350_303, 75_350_303, 0),
    (74_051_160, 74_051_160, 0),
    (72_796_055, 72_796_055, 0),
    (71_582_788, 71_582_788, 0),
    (70_409_299, 70_409_299, 0),
    (69_273_666, 69_273_666, 0),
    (68_174_084, 68_174_084, 0),
    (-0x8000_0000, 0, 5),
];

/// A compact list of integers with the given number of bits per entry.
#[derive(Clone, Debug, Default)]
pub struct BitStorage {
    data: Vec<u64>,
    bits: usize,
    mask: u64,
    size: usize,
    values_per_long: usize,
    divide_mul: u64,
    divide_add: u64,
    divide_shift: i32,
}

#[derive(Debug)]
pub enum BitStorageError {
    InvalidLength { got: usize, expected: usize },
}
impl fmt::Display for BitStorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { got, expected } => write!(
                f,
                "Invalid length given for storage, got: {got}, but expected: {expected}",
            ),
        }
    }
}
impl Error for BitStorageError {}

impl BitStorage {
    #[must_use]
    pub fn into_data(self) -> Vec<u64> {
        self.data
    }

    /// Create a new `BitStorage` with the given number of bits per entry.
    /// `size` is the number of entries in the `BitStorage`.
    pub fn new(bits: usize, size: usize, data: Option<Vec<u64>>) -> Result<Self, BitStorageError> {
        if let Some(data) = &data {
            // 0 bit storage
            if data.is_empty() {
                return Ok(Self {
                    data: Vec::new(),
                    bits,
                    size,
                    ..Default::default()
                });
            }
        }

        debug_assert!((1..=32).contains(&bits), "{bits} is not in 1..=32");

        let values_per_long = 64 / bits;
        let magic_index = values_per_long - 1;
        let (divide_mul, divide_add, divide_shift) = MAGIC[magic_index];
        let calculated_length = size.div_ceil(values_per_long);

        let mask = (1 << bits) - 1;

        let using_data = if let Some(data) = data {
            if data.len() != calculated_length {
                return Err(BitStorageError::InvalidLength {
                    got: data.len(),
                    expected: calculated_length,
                });
            }
            data
        } else {
            vec![0; calculated_length]
        };

        #[expect(clippy::cast_sign_loss, reason = "the sign is not relevant")]
        let (divide_mul, divide_add) = {
            let divide_mul = divide_mul as u32;
            let divide_add = divide_add as u32;
            (divide_mul, divide_add)
        };

        Ok(Self {
            data: using_data,
            bits,
            mask,
            size,
            values_per_long,
            divide_mul: u64::from(divide_mul),
            divide_add: u64::from(divide_add),
            divide_shift,
        })
    }

    #[must_use]
    pub const fn cell_index(&self, index: u64) -> usize {
        // as unsigned wrap
        let first = self.divide_mul;
        let second = self.divide_add;

        (((index * first) + second) >> 32 >> self.divide_shift) as usize
    }

    /// Get the data at the given index.
    ///
    /// # Panics
    ///
    /// This function will panic if the given index is greater than or equal to
    /// the size of this storage.
    #[must_use]
    pub fn get(&self, index: usize) -> u64 {
        assert!(
            index < self.size,
            "Index {index} out of bounds (must be less than {})",
            self.size
        );

        // 0 bit storage
        if self.data.is_empty() {
            return 0;
        }

        let cell_index = self.cell_index(index as u64);
        let cell = &self.data[cell_index];
        let bit_index = (index - cell_index * self.values_per_long) * self.bits;
        (cell >> bit_index) & self.mask
    }

    pub fn get_and_set(&mut self, index: usize, value: u64) -> u64 {
        // 0 bit storage
        if self.data.is_empty() {
            return 0;
        }

        debug_assert!(index < self.size);
        debug_assert!(value <= self.mask);
        let cell_index = self.cell_index(index as u64);
        let cell = &mut self.data[cell_index];
        let bit_index = (index - cell_index * self.values_per_long) * self.bits;
        let old_value = (*cell >> (bit_index as u64)) & self.mask;
        *cell = *cell & !(self.mask << bit_index) | ((value & self.mask) << bit_index);
        old_value
    }

    pub fn set(&mut self, index: usize, value: u64) {
        // 0 bit storage
        if self.data.is_empty() {
            return;
        }

        debug_assert!(index < self.size);
        debug_assert!(value <= self.mask);
        let cell_index = self.cell_index(index as u64);
        let cell = &mut self.data[cell_index];
        let bit_index = (index - cell_index * self.values_per_long) * self.bits;
        *cell = *cell & !(self.mask << bit_index) | ((value & self.mask) << bit_index);
    }

    /// The number of entries.
    #[inline]
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    #[must_use]
    pub const fn iter(&self) -> BitStorageIter<'_> {
        BitStorageIter {
            storage: self,
            index: 0,
        }
    }
}

impl<'a> IntoIterator for &'a BitStorage {
    type IntoIter = BitStorageIter<'a>;
    type Item = u64;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct BitStorageIter<'a> {
    storage: &'a BitStorage,
    index: usize,
}

impl Iterator for BitStorageIter<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.storage.size {
            return None;
        }

        let value = self.storage.get(self.index);
        self.index += 1;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wikivg_example() {
        let data = [
            1, 2, 2, 3, 4, 4, 5, 6, 6, 4, 8, 0, 7, 4, 3, 13, 15, 16, 9, 14, 10, 12, 0, 2,
        ];
        let compact_data: [u64; 2] = [0x0020_8631_4841_8841, 0x0101_8A72_60F6_8C87];
        let storage = BitStorage::new(5, data.len(), Some(compact_data.to_vec())).unwrap();

        for (i, expected) in data.iter().enumerate() {
            assert_eq!(storage.get(i), *expected);
        }
    }
}
