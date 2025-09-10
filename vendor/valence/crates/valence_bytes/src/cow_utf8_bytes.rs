use std::fmt::{Display, Formatter};
use std::ops::Deref;

use crate::Utf8Bytes;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CowUtf8Bytes<'a> {
    Borrowed(&'a str),
    Owned(Utf8Bytes),
}

impl<'a> CowUtf8Bytes<'a> {
    pub fn into_owned(self) -> Utf8Bytes {
        match self {
            Self::Borrowed(bytes) => Utf8Bytes::copy_from_str(bytes),
            Self::Owned(bytes) => bytes,
        }
    }

    pub fn as_str(&self) -> &str {
        self
    }
}

impl<'a> Display for CowUtf8Bytes<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

impl<'a> Default for CowUtf8Bytes<'a> {
    fn default() -> Self {
        Utf8Bytes::default().into()
    }
}

impl<'a> Deref for CowUtf8Bytes<'a> {
    type Target = str;

    fn deref(&self) -> &str {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}

impl<'a> From<&'a str> for CowUtf8Bytes<'a> {
    fn from(bytes: &'a str) -> Self {
        Self::Borrowed(bytes)
    }
}

impl<'a> From<Utf8Bytes> for CowUtf8Bytes<'a> {
    fn from(bytes: Utf8Bytes) -> Self {
        Self::Owned(bytes)
    }
}
