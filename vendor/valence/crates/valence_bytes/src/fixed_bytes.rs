use std::convert::AsRef;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Deref;

use bytes::Bytes;

/// Wrapper around [`bytes::Bytes`] which guarantees that the bytes are exactly
/// `N` length
#[derive(Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Debug)]
pub struct FixedBytes<const N: usize>(Bytes);

impl<const N: usize> FixedBytes<N> {
    pub fn new(value: Bytes) -> Option<Self> {
        if value.len() == N {
            // SAFETY: The length of the bytes has been checked
            Some(unsafe { FixedBytes::new_unchecked(value) })
        } else {
            None
        }
    }

    /// # Safety
    /// `bytes` must have a length of exactly `N`
    pub const unsafe fn new_unchecked(bytes: Bytes) -> Self {
        FixedBytes(bytes)
    }

    pub const fn from_static(value: &'static [u8; N]) -> Self {
        // SAFETY: Length is guaranteed to be `N`
        unsafe { Self::new_unchecked(Bytes::from_static(value)) }
    }

    pub fn copy_from_array(value: &[u8; N]) -> Self {
        // SAFETY: Length is guaranteed to be `N`
        unsafe { Self::new_unchecked(Bytes::copy_from_slice(value)) }
    }
}

impl<const N: usize> Deref for FixedBytes<N> {
    type Target = [u8; N];

    fn deref(&self) -> &[u8; N] {
        let data: &[u8] = &self.0;
        // SAFETY: FixedBytes is guaranteed to have a length of exactly `N` bytes
        let data: &[u8; N] = unsafe { data.try_into().unwrap_unchecked() };
        data
    }
}

impl<const N: usize> AsRef<[u8; N]> for FixedBytes<N> {
    fn as_ref(&self) -> &[u8; N] {
        self
    }
}

impl<const N: usize> AsRef<[u8]> for FixedBytes<N> {
    fn as_ref(&self) -> &[u8] {
        let value: &[u8; N] = self;
        value
    }
}

impl<const N: usize> AsRef<Bytes> for FixedBytes<N> {
    fn as_ref(&self) -> &Bytes {
        &self.0
    }
}

#[derive(Debug, Copy, Clone)]
pub struct TryFromBytesError(());

impl Display for TryFromBytesError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "could not convert Bytes to FixedBytes because it is the wrong length"
        )
    }
}

impl Error for TryFromBytesError {}

impl<const N: usize> TryFrom<Bytes> for FixedBytes<N> {
    type Error = TryFromBytesError;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Self::new(value).ok_or(TryFromBytesError(()))
    }
}

impl<const N: usize> From<[u8; N]> for FixedBytes<N> {
    fn from(value: [u8; N]) -> Self {
        FixedBytes::copy_from_array(&value)
    }
}

impl<const N: usize> From<&'static [u8; N]> for FixedBytes<N> {
    fn from(value: &'static [u8; N]) -> Self {
        Self::from_static(value)
    }
}
