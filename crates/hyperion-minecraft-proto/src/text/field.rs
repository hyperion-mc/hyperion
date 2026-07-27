//! Reading typed values out of a decoded compound.
//!
//! Each helper is one DFU combinator: `fieldOf` is [`required`],
//! `optionalFieldOf` is [`optional`], and `optionalFieldOf(name, default)` is
//! the `_or` pair. Absent means absent; a present field of the wrong type is
//! an error rather than a silent default, matching `DataResult`.

use std::borrow::Cow;

use crate::{
    Error, Result,
    nbt::{Compound, Tag},
};

pub(super) fn required<'a, 'c>(
    compound: &'c Compound<'a>,
    field: &'static str,
) -> Result<&'c Tag<'a>> {
    compound.get(field).ok_or(Error::MissingField(field))
}

pub(super) fn string<'a>(compound: &Compound<'a>, field: &'static str) -> Result<Cow<'a, str>> {
    as_string(required(compound, field)?, field)
}

pub(super) fn optional_string<'a>(
    compound: &Compound<'a>,
    field: &'static str,
) -> Result<Option<Cow<'a, str>>> {
    compound
        .get(field)
        .map(|tag| as_string(tag, field))
        .transpose()
}

pub(super) fn int(compound: &Compound<'_>, field: &'static str) -> Result<i32> {
    as_int(required(compound, field)?, field)
}

pub(super) fn optional_int(compound: &Compound<'_>, field: &'static str) -> Result<Option<i32>> {
    compound
        .get(field)
        .map(|tag| as_int(tag, field))
        .transpose()
}

pub(super) fn int_or(compound: &Compound<'_>, field: &'static str, default: i32) -> Result<i32> {
    Ok(optional_int(compound, field)?.unwrap_or(default))
}

pub(super) fn optional_bool(compound: &Compound<'_>, field: &'static str) -> Result<Option<bool>> {
    compound
        .get(field)
        .map(|tag| {
            tag.as_bool().ok_or_else(|| Error::WrongTagType {
                field,
                expected: "TAG_Byte",
                found: tag.id(),
            })
        })
        .transpose()
}

pub(super) fn bool_or(compound: &Compound<'_>, field: &'static str, default: bool) -> Result<bool> {
    Ok(optional_bool(compound, field)?.unwrap_or(default))
}

pub(super) fn compound<'a, 'c>(
    compound: &'c Compound<'a>,
    field: &'static str,
) -> Result<&'c Compound<'a>> {
    let tag = required(compound, field)?;
    tag.as_compound().ok_or_else(|| Error::WrongTagType {
        field,
        expected: "TAG_Compound",
        found: tag.id(),
    })
}

fn as_string<'a>(tag: &Tag<'a>, field: &'static str) -> Result<Cow<'a, str>> {
    match tag {
        Tag::String(value) => Ok(value.clone()),
        other => Err(Error::WrongTagType {
            field,
            expected: "TAG_String",
            found: other.id(),
        }),
    }
}

const fn as_int(tag: &Tag<'_>, field: &'static str) -> Result<i32> {
    match tag {
        Tag::Int(value) => Ok(*value),
        other => Err(Error::WrongTagType {
            field,
            expected: "TAG_Int",
            found: other.id(),
        }),
    }
}
