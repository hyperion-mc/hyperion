use std::convert::AsRef;
use std::fmt::{Debug, Display, Formatter};
use std::ops::Deref;

use bytes::Bytes;

/// Wrapper around [`bytes::Bytes`] which guarantees that the bytes are UTF-8
#[derive(Default, Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct Utf8Bytes(Bytes);

impl Utf8Bytes {
    pub fn new(value: Bytes) -> Result<Self, std::str::Utf8Error> {
        // Check that the bytes are UTF-8
        std::str::from_utf8(&value)?;

        // SAFETY: Bytes have been confirmed to be UTF-8
        Ok(unsafe { Utf8Bytes::new_unchecked(value) })
    }

    /// # Safety
    /// `bytes` must be UTF-8
    pub const unsafe fn new_unchecked(bytes: Bytes) -> Self {
        Utf8Bytes(bytes)
    }

    pub const fn from_static(value: &'static str) -> Self {
        // SAFETY: str guarantees UTF-8
        unsafe { Self::new_unchecked(Bytes::from_static(value.as_bytes())) }
    }

    pub fn copy_from_str(value: &str) -> Self {
        // SAFETY: str guarantees UTF-8
        unsafe { Self::new_unchecked(Bytes::copy_from_slice(value.as_bytes())) }
    }

    pub fn as_str(&self) -> &str {
        self
    }
}

impl Deref for Utf8Bytes {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: Utf8Bytes is guaranteed to be UTF-8
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }
}

impl AsRef<str> for Utf8Bytes {
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsRef<[u8]> for Utf8Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<Bytes> for Utf8Bytes {
    fn as_ref(&self) -> &Bytes {
        &self.0
    }
}

impl TryFrom<Bytes> for Utf8Bytes {
    type Error = std::str::Utf8Error;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<String> for Utf8Bytes {
    fn from(value: String) -> Self {
        // SAFETY: String guarantees UTF-8
        unsafe { Utf8Bytes::new_unchecked(value.into()) }
    }
}

impl From<&'static str> for Utf8Bytes {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl Debug for Utf8Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value: &str = self;
        Debug::fmt(value, f)
    }
}

impl Display for Utf8Bytes {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value: &str = self;
        Display::fmt(value, f)
    }
}
