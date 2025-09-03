use std::{clone::Clone, iter::FusedIterator};

use roaring::RoaringBitmap;
use valence_protocol::BlockState;

use crate::indirect::Indirect;

const LEN: usize = 4096;
const HALF_LEN: usize = LEN >> 1;

mod indirect;

mod encode;

type Data = u16;

#[derive(Clone, Debug)]
pub enum PalettedContainer {
    Single(Data),
    Indirect(Indirect),
    Direct(Box<[Data]>),
}

impl PalettedContainer {
    pub fn fill(&mut self, value: Data) {
        *self = Self::Single(value);
    }

    #[allow(clippy::missing_safety_doc)]
    #[must_use]
    pub unsafe fn get_unchecked(&self, index: usize) -> Data {
        match self {
            Self::Single(data) => *data,
            Self::Indirect(indirect) => unsafe { indirect.get_unchecked(index) },
            Self::Direct(direct) => unsafe { *direct.get_unchecked(index) },
        }
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Data {
        assert!(index < LEN);
        unsafe { self.get_unchecked(index) }
    }

    /// Returns the previous value
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn set_unchecked(&mut self, index: usize, value: Data) -> Data {
        match self {
            Self::Single(previous) => {
                if *previous == value {
                    return *previous;
                }

                let mut indirect = Indirect::from_single(*previous);
                let _ = unsafe { indirect.set_unchecked(index, value) }; // error can never occur
                let previous = *previous;
                *self = Self::Indirect(indirect);
                previous
            }
            Self::Indirect(indirect) => match unsafe { indirect.set_unchecked(index, value) } {
                Ok(previous) => previous,
                Err(indirect::Full) => {
                    let mut direct = indirect.to_direct();
                    let ptr = unsafe { direct.get_unchecked_mut(index) };
                    let previous = *ptr;
                    *ptr = value;
                    *self = Self::Direct(direct);
                    previous
                }
            },
            Self::Direct(direct) => {
                let ptr = unsafe { direct.get_unchecked_mut(index) };
                let previous = *ptr;
                *ptr = value;
                previous
            }
        }
    }

    /// Returns the number of unique block states in this container.
    /// This operation is O(1) for Single and Indirect variants,
    /// and O(n) for Direct variant where n is the number of blocks.
    #[must_use]
    pub fn unique_count(&self) -> usize {
        match self {
            // Single always has exactly 1 unique value
            Self::Single(_) => 1,
            // Indirect stores the palette length directly
            Self::Indirect(indirect) => indirect.palette_len as usize,
            // Direct needs to count unique values
            Self::Direct(direct) => {
                // Use a simple bit array since we know values are u16
                let mut seen = RoaringBitmap::new();
                for value in direct.iter().copied() {
                    seen.insert(u32::from(value));
                }
                unsafe { usize::try_from(seen.len()).unwrap_unchecked() }
            }
        }
    }

    /// Returns an iterator over unique block states in this container.
    /// The iterator is efficient for all variants:
    /// - Single: yields exactly one value
    /// - Indirect: yields from the palette
    /// - Direct: uses a [`RoaringBitmap`] to track unique values
    pub fn unique_blocks(&self) -> UniqueBlockIter<'_> {
        match self {
            Self::Single(value) => UniqueBlockIter {
                inner: Box::new(std::iter::once(*value)),
            },
            Self::Indirect(indirect) => UniqueBlockIter {
                inner: Box::new(
                    indirect.palette[..indirect.palette_len as usize]
                        .iter()
                        .copied(),
                ),
            },
            Self::Direct(direct) => {
                // Use RoaringBitmap for unique values since we know they're u16
                let mut seen = RoaringBitmap::new();
                for value in direct.iter().copied() {
                    seen.insert(u32::from(value));
                }

                UniqueBlockIter {
                    inner: Box::new(
                        seen.into_iter()
                            .map(|x| unsafe { u16::try_from(x).unwrap_unchecked() }),
                    ),
                }
            }
        }
    }

    #[must_use]
    pub fn instances_of(&self, block: BlockState) -> Box<dyn Iterator<Item = usize> + '_> {
        let block = block.to_raw();
        match self {
            Self::Single(value) => {
                if *value == block {
                    Box::new(0..LEN)
                } else {
                    Box::new(std::iter::empty())
                }
            }
            Self::Indirect(indirect) => {
                let palette_idx = indirect.palette[..indirect.palette_len as usize]
                    .iter()
                    .position(|&x| x == block);

                #[allow(clippy::option_if_let_else)]
                if let Some(idx) = palette_idx {
                    Box::new(
                        indirect
                            .indices()
                            .enumerate()
                            .filter_map(
                                move |(i, value)| {
                                    if value as usize == idx { Some(i) } else { None }
                                },
                            ),
                    )
                } else {
                    Box::new(std::iter::empty())
                }
            }
            Self::Direct(direct) => Box::new(
                direct
                    .iter()
                    .enumerate()
                    .filter_map(move |(i, &value)| if value == block { Some(i) } else { None }),
            ),
        }
    }
}

#[must_use]
pub struct PaletteIter<'a> {
    container: &'a PalettedContainer,
    index: usize,
}

pub struct PaletteIntoIter {
    container: PalettedContainer,
    index: usize,
}

#[must_use]
pub struct UniqueBlockIter<'a> {
    inner: Box<dyn Iterator<Item = Data> + 'a>,
}

impl Iterator for PaletteIter<'_> {
    type Item = Data;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= LEN {
            return None;
        }
        let value = unsafe { self.container.get_unchecked(self.index) };
        self.index += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = LEN - self.index;
        (remaining, Some(remaining))
    }
}

impl Iterator for PaletteIntoIter {
    type Item = Data;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= LEN {
            return None;
        }
        let value = unsafe { self.container.get_unchecked(self.index) };
        self.index += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = LEN - self.index;
        (remaining, Some(remaining))
    }
}

impl Iterator for UniqueBlockIter<'_> {
    type Item = Data;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl PalettedContainer {
    pub fn iter(&self) -> PaletteIter<'_> {
        <&Self as IntoIterator>::into_iter(self)
    }
}

impl<'a> IntoIterator for &'a PalettedContainer {
    type IntoIter = PaletteIter<'a>;
    type Item = Data;

    fn into_iter(self) -> Self::IntoIter {
        PaletteIter {
            container: self,
            index: 0,
        }
    }
}

impl IntoIterator for PalettedContainer {
    type IntoIter = PaletteIntoIter;
    type Item = Data;

    fn into_iter(self) -> Self::IntoIter {
        PaletteIntoIter {
            container: self,
            index: 0,
        }
    }
}

impl ExactSizeIterator for PaletteIter<'_> {
    fn len(&self) -> usize {
        LEN - self.index
    }
}

impl FusedIterator for PaletteIter<'_> {}

impl ExactSizeIterator for PaletteIntoIter {
    fn len(&self) -> usize {
        LEN - self.index
    }
}

impl FusedIterator for PaletteIntoIter {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unique_count() {
        // Test Single
        let single = PalettedContainer::Single(5);
        assert_eq!(single.unique_count(), 1);

        // Test Indirect
        let mut indirect = Indirect::from_single(1);
        unsafe {
            indirect.set_unchecked(1, 2).unwrap();
            indirect.set_unchecked(2, 3).unwrap();
        }
        let indirect = PalettedContainer::Indirect(indirect);
        assert_eq!(indirect.unique_count(), 3);

        // Test Direct
        let mut direct = vec![1_u16; LEN].into_boxed_slice();
        direct[0] = 2;
        direct[1] = 3;
        direct[2] = 2;
        let direct = PalettedContainer::Direct(direct);
        assert_eq!(direct.unique_count(), 3);
    }

    #[test]
    fn test_unique_blocks_iterator() {
        // Test Single
        let single = PalettedContainer::Single(5);
        let unique: Vec<_> = single.unique_blocks().collect();
        assert_eq!(unique, vec![5]);

        // Test Indirect
        let mut indirect = Indirect::from_single(1);
        unsafe {
            indirect.set_unchecked(1, 2).unwrap();
            indirect.set_unchecked(2, 3).unwrap();
        }
        let indirect = PalettedContainer::Indirect(indirect);
        let mut unique: Vec<_> = indirect.unique_blocks().collect();
        unique.sort_unstable();
        assert_eq!(unique, vec![1, 2, 3]);

        // Test Direct
        let mut direct = vec![1_u16; LEN].into_boxed_slice();
        direct[0] = 2;
        direct[1] = 3;
        direct[2] = 2;
        let direct = PalettedContainer::Direct(direct);
        let mut unique: Vec<_> = direct.unique_blocks().collect();
        unique.sort_unstable();
        assert_eq!(unique, vec![1, 2, 3]);
    }
}
