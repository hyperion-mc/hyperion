use std::simd::{Simd, cmp::SimdPartialEq};

use crate::{Data, HALF_LEN, LEN};

#[derive(Clone, Debug)]
pub struct Indirect {
    pub palette: [Data; 16],
    pub palette_len: u8,
    data: Box<[u8; HALF_LEN]>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Full;

impl Indirect {
    pub fn from_single(data: Data) -> Self {
        let mut palette = [Data::default(); 16];
        palette[0] = data;

        Self {
            palette,
            palette_len: 1,
            data: Box::new([0; HALF_LEN]),
        }
    }

    pub fn index_of(&self, data: Data) -> Option<u8> {
        // Create a SIMD vector filled with the search element
        let search_simd: Simd<u16, 16> = Simd::splat(data);

        // todo: is this zero cost?
        let chunk_simd: Simd<Data, 16> = Simd::from_array(self.palette);

        // Compare the chunk with the search element
        let mask = chunk_simd.simd_eq(search_simd);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "idx is [0, 16) which is a valid u8"
        )]
        mask.first_set().map(|idx| idx as u8)
    }

    pub unsafe fn get_unchecked(&self, index: usize) -> Data {
        debug_assert!(index < LEN, "index {index} is out of bounds");

        let packed_byte_index = index / 2;

        #[expect(
            clippy::indexing_slicing,
            reason = "if debug_assert passes, then index is in bounds"
        )]
        let packed_byte = self.data[packed_byte_index];

        // Create a mask based on whether the index is even or odd
        // Either 0 or 4
        let shift = (index & 1) << 2;

        // Shift and mask to get the correct nibble (u4)
        let palette_index = (packed_byte >> shift) & 0x0F;

        #[expect(
            clippy::indexing_slicing,
            reason = "u4 is at most 16, palette is of size 16 so this is valid"
        )]
        self.palette[palette_index as usize]
    }

    pub fn get(&self, index: usize) -> Option<Data> {
        if index < LEN {
            Some(unsafe { self.get_unchecked(index) })
        } else {
            None
        }
    }

    pub unsafe fn set_unchecked(&mut self, index: usize, value: Data) -> Result<Data, Full> {
        debug_assert!(index < LEN, "index {index} is out of bounds");

        let palette_index = match self.index_of(value) {
            Some(idx) => idx,
            None => {
                if self.palette_len == 16 {
                    return Err(Full);
                }
                let new_index = self.palette_len;
                self.palette[new_index as usize] = value;
                self.palette_len += 1;
                new_index
            }
        };

        let packed_byte_index = index / 2;
        let shift = (index & 1) << 2;
        let mask = 0xF << shift;

        let ptr = unsafe { self.data.get_unchecked_mut(packed_byte_index) };

        let other_part = *ptr & !mask;
        let previous = (*ptr & mask) >> shift;

        *ptr = other_part | ((palette_index) << shift);

        let previous = self.palette[previous as usize];

        Ok(previous)
    }

    pub fn indices(&self) -> impl Iterator<Item = u8> + '_ {
        (0..LEN).map(|i| self.index_of(unsafe { self.get_unchecked(i) }).unwrap())
    }

    pub fn to_direct(&self) -> Box<[Data]> {
        let mut direct = Box::new_uninit_slice(LEN);

        for i in 0..LEN {
            unsafe {
                direct[i].write(self.get_unchecked(i));
            }
        }

        unsafe { direct.assume_init() }
    }
}
