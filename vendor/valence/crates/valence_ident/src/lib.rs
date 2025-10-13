#![doc = include_str!("../README.md")]

use std::borrow::Borrow;
use std::fmt;
use std::fmt::Formatter;

use bytes::Bytes;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
/// Used internally by the `ident` macro. Not public API.
#[doc(hidden)]
pub use valence_bytes::Utf8Bytes;
/// Used internally by the `ident` macro. Not public API.
#[doc(hidden)]
pub use valence_ident_macros::parse_ident_str;

/// Creates a new [`Ident`] at compile time from a string literal. A compile
/// error is raised if the string is not a valid resource identifier.
///
/// The type of the expression returned by this macro is `Ident`.
/// The expression is usable in a `const` context.
///
/// # Examples
///
/// ```
/// # use valence_ident::{ident, Ident};
/// let my_ident: Ident = ident!("apple");
///
/// println!("{my_ident}");
/// ```
#[macro_export]
macro_rules! ident {
    ($string:literal) => {
        // SAFETY: parse_ident_str returns a &'static str, which is guaranteed to be
        // valid UTF-8
        $crate::Ident::new_unchecked($crate::Utf8Bytes::from_static($crate::parse_ident_str!(
            $string
        )))
    };
}

/// A wrapper around [`Utf8Bytes`] which guarantees the wrapped string is a
/// valid resource identifier.
///
/// A resource identifier is a string divided into a "namespace" part and a
/// "path" part. For instance `minecraft:apple` and `valence:frobnicator` are
/// both valid identifiers. A string must match the regex
/// `^([a-z0-9_.-]+:)?[a-z0-9_.-\/]+$` to be successfully parsed.
///
/// While parsing, if the namespace part is left off (the part before and
/// including the colon) then "minecraft:" is inserted at the beginning of the
/// string.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ident {
    string: Utf8Bytes,
}

/// The error type created when an [`Ident`] cannot be parsed from a
/// string. Contains the string that failed to parse.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Error)]
#[error("invalid resource identifier \"{0}\"")]
pub struct IdentError(pub Utf8Bytes);

impl Ident {
    pub fn is_valid(string: &str) -> bool {
        match string.split_once(':') {
            Some((namespace, path)) => check_namespace(namespace) && check_path(path),
            None => check_path(string),
        }
    }

    pub fn new(string: impl Into<Utf8Bytes>) -> Result<Ident, IdentError> {
        parse(string.into())
    }

    /// Used internally by the `ident` macro. Not public API.
    #[doc(hidden)]
    pub const fn new_unchecked(string: Utf8Bytes) -> Self {
        Self { string }
    }

    pub fn as_str(&self) -> &str {
        &self.string
    }

    pub fn into_inner(self) -> Utf8Bytes {
        self.string
    }

    /// Returns the namespace part of this resource identifier (the part before
    /// the colon).
    pub fn namespace(&self) -> &str {
        self.namespace_and_path().0
    }

    /// Returns the path part of this resource identifier (the part after the
    /// colon).
    pub fn path(&self) -> &str {
        self.namespace_and_path().1
    }

    pub fn namespace_and_path(&self) -> (&str, &str) {
        self.as_str()
            .split_once(':')
            .expect("invalid resource identifier")
    }
}

fn check_namespace(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'))
}

fn check_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-' | '/'))
}

fn parse(string: Utf8Bytes) -> Result<Ident, IdentError> {
    match string.split_once(':') {
        Some((namespace, path)) if check_namespace(namespace) && check_path(path) => {
            Ok(Ident::new_unchecked(string))
        }
        None if check_path(&string) => {
            let string = format!("minecraft:{string}").into();
            Ok(Ident::new_unchecked(string))
        }
        _ => Err(IdentError(string)),
    }
}

impl AsRef<str> for Ident {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Ident {
    fn borrow(&self) -> &str {
        self.as_ref()
    }
}

impl From<Ident> for String {
    fn from(value: Ident) -> Self {
        value.as_str().to_owned()
    }
}

impl TryFrom<Utf8Bytes> for Ident {
    type Error = IdentError;

    fn try_from(value: Utf8Bytes) -> Result<Self, Self::Error> {
        parse(value)
    }
}

// TODO:
// impl FromStr for Ident {
//     type Err = IdentError;
//
//     fn from_str(s: &str) -> Result<Self, Self::Err> {
//         Ok(Ident::new(s)?.into())
//     }
// }
//
// impl<'a> TryFrom<&'a str> for Ident {
//     type Error = IdentError;
//
//     fn try_from(value: &'a str) -> Result<Self, Self::Error> {
//         Ok(Ident::new(value)?.into())
//     }
// }
//
// impl TryFrom<String> for Ident {
//     type Error = IdentError;
//
//     fn try_from(value: String) -> Result<Self, Self::Error> {
//         Ok(Ident::new(value)?.into())
//     }
// }
//
// impl<'a> TryFrom<Cow<'a, str>> for Ident {
//     type Error = IdentError;
//
//     fn try_from(value: Cow<'a, str>) -> Result<Self, Self::Error> {
//         Ok(Ident::new(value)?.into())
//     }
// }

impl fmt::Debug for Ident {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl Serialize for Ident {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.string.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Ident {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Bytes::from_owner(String::deserialize(deserializer)?);
        let bytes = Utf8Bytes::try_from(bytes).map_err(D::Error::custom)?;
        Ident::try_from(bytes).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_namespace_and_path() {
        let id = ident!("namespace:path");
        assert_eq!(id.namespace(), "namespace");
        assert_eq!(id.path(), "path");
    }

    #[test]
    fn parse_valid() {
        ident!("minecraft:whatever");
        ident!("_what-ever55_:.whatever/whatever123456789_");
        ident!("valence:frobnicator");
    }

    #[test]
    #[should_panic]
    fn parse_invalid_0() {
        Ident::new("").unwrap();
    }

    #[test]
    #[should_panic]
    fn parse_invalid_1() {
        Ident::new(":").unwrap();
    }

    #[test]
    #[should_panic]
    fn parse_invalid_2() {
        Ident::new("foo:bar:baz").unwrap();
    }

    #[test]
    fn equality() {
        assert_eq!(ident!("minecraft:my.identifier"), ident!("my.identifier"));
    }
}
