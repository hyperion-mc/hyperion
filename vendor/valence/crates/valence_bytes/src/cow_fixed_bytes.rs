use std::ops::Deref;

use crate::FixedBytes;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CowFixedBytes<'a, const N: usize> {
    Borrowed(&'a [u8; N]),
    Owned(FixedBytes<N>),
}

impl<'a, const N: usize> CowFixedBytes<'a, N> {
    pub fn into_owned(self) -> FixedBytes<N> {
        match self {
            Self::Borrowed(bytes) => FixedBytes::copy_from_array(bytes),
            Self::Owned(bytes) => bytes,
        }
    }
}

impl<'a, const N: usize> Deref for CowFixedBytes<'a, N> {
    type Target = [u8; N];

    fn deref(&self) -> &[u8; N] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for CowFixedBytes<'a, N> {
    fn from(bytes: &'a [u8; N]) -> Self {
        Self::Borrowed(bytes)
    }
}

impl<'a, const N: usize> From<FixedBytes<N>> for CowFixedBytes<'a, N> {
    fn from(bytes: FixedBytes<N>) -> Self {
        Self::Owned(bytes)
    }
}
