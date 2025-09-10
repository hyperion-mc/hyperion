use std::ops::{Deref, RangeBounds};

use crate::Bytes;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CowBytes<'a> {
    Borrowed(&'a [u8]),
    Owned(Bytes),
}

impl<'a> CowBytes<'a> {
    pub fn into_owned(self) -> Bytes {
        match self {
            Self::Borrowed(bytes) => Bytes::copy_from_slice(bytes),
            Self::Owned(bytes) => bytes,
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::Borrowed(bytes) => {
                *bytes = &bytes[0..0];
            }
            Self::Owned(bytes) => bytes.clear(),
        }
    }

    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        match self {
            Self::Borrowed(bytes) => {
                Self::Borrowed(&bytes[(range.start_bound().cloned(), range.end_bound().cloned())])
            }
            Self::Owned(bytes) => Self::Owned(bytes.slice(range)),
        }
    }

    pub fn split_off(&mut self, at: usize) -> Self {
        assert!(at <= self.len());
        let result = self.slice(at..);
        self.truncate(at);
        result
    }

    pub fn split_to(&mut self, at: usize) -> Self {
        assert!(at <= self.len());
        let mut result = self.clone();
        result.truncate(at);
        *self = self.slice(at..);
        result
    }

    pub fn truncate(&mut self, len: usize) {
        match self {
            Self::Borrowed(bytes) => {
                if let Some(truncated) = bytes.get(0..len) {
                    *bytes = truncated;
                }
            }
            Self::Owned(bytes) => bytes.truncate(len),
        }
    }
}

impl<'a> Default for CowBytes<'a> {
    fn default() -> Self {
        Bytes::default().into()
    }
}

impl<'a> Deref for CowBytes<'a> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}

impl<'a> From<&'a [u8]> for CowBytes<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self::Borrowed(bytes)
    }
}

impl<'a> From<Bytes> for CowBytes<'a> {
    fn from(bytes: Bytes) -> Self {
        Self::Owned(bytes)
    }
}
