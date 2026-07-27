//! Java's modified UTF-8, which is what NBT strings are.
//!
//! `StringTag.write` calls `DataOutput.writeUTF`, so an NBT string is not UTF-8:
//! `U+0000` travels as the two bytes `C0 80`, and a supplementary character
//! travels as its two UTF-16 surrogates at three bytes each rather than as one
//! four-byte sequence. The length prefix is an unsigned short counting encoded
//! bytes, which is why NBT strings cap out well below the protocol's own string
//! limit.

use std::borrow::Cow;

use crate::{Error, Reader, Result, Writer};

/// Longest encoding the unsigned-short prefix can describe.
pub const MAX_ENCODED_LENGTH: usize = 65_535;

/// Append `value` the way `DataOutput.writeUTF` would.
///
/// # Errors
/// Returns [`Error::StringTooLong`] when the encoding needs more than
/// [`MAX_ENCODED_LENGTH`] bytes, which is where Java throws
/// `UTFDataFormatException`.
pub fn write(writer: &mut Writer, value: &str) -> Result<()> {
    let length = encoded_length(value);
    let Ok(prefix) = u16::try_from(length) else {
        return Err(Error::StringTooLong {
            length,
            max: MAX_ENCODED_LENGTH,
        });
    };
    writer.u16(prefix);

    // A supplementary character is the only reason UTF-8 and modified UTF-8
    // disagree on a non-null string, and it always begins with a byte at or
    // above 0xF0, so this scan decides the whole string in one pass.
    if value.bytes().all(|byte| byte != 0 && byte < 0xF0) {
        writer.raw(value.as_bytes());
        return Ok(());
    }

    for character in value.chars() {
        let code_point = u32::from(character);
        match code_point {
            // U+0000 goes out as two bytes so that no null appears in the
            // encoding, which is the whole point of the "modified" in the name.
            0 | 0x0080..=0x07FF => write_two(writer, code_point),
            0x0001..=0x007F => writer.u8(low_byte(code_point)),
            0x0800..=0xFFFF => write_three(writer, code_point),
            _ => {
                let offset = code_point - 0x1_0000;
                write_three(writer, 0xD800 + (offset >> 10));
                write_three(writer, 0xDC00 + (offset & 0x3FF));
            }
        }
    }
    Ok(())
}

/// Read a string the way `DataInput.readUTF` would.
///
/// Borrows the input when its bytes are already valid UTF-8 meaning the same
/// text, which covers every string that carries no null and no supplementary
/// character — in practice almost all of them.
///
/// # Errors
/// Returns [`Error::InvalidModifiedUtf8`] on a malformed sequence or on an
/// unpaired surrogate, and [`Error::UnexpectedEof`] on truncated input.
pub fn read<'a>(reader: &mut Reader<'a>) -> Result<Cow<'a, str>> {
    let length = usize::from(reader.u16()?);
    decode(reader.take(length)?)
}

fn decode(bytes: &[u8]) -> Result<Cow<'_, str>> {
    if bytes.iter().all(|byte| *byte < 0xF0) {
        if let Ok(text) = std::str::from_utf8(bytes) {
            return Ok(Cow::Borrowed(text));
        }
    }
    decode_surrogates(bytes).map(Cow::Owned)
}

/// The path for strings whose bytes are not also their UTF-8 spelling.
///
/// Java hands back a `String` of UTF-16 code units and never inspects them, so
/// it accepts an unpaired surrogate; `str` cannot hold one, so this rejects
/// input the server would have taken. Nothing vanilla emits such a string.
fn decode_surrogates(bytes: &[u8]) -> Result<String> {
    let mut out = String::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        let unit = read_unit(bytes, &mut position)?;
        let code_point = match unit {
            0xD800..=0xDBFF => {
                let low = read_unit(bytes, &mut position)?;
                if !(0xDC00..=0xDFFF).contains(&low) {
                    return Err(Error::InvalidModifiedUtf8);
                }
                0x1_0000 + ((unit - 0xD800) << 10) + (low - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(Error::InvalidModifiedUtf8),
            _ => unit,
        };
        out.push(char::from_u32(code_point).ok_or(Error::InvalidModifiedUtf8)?);
    }
    Ok(out)
}

/// Decode one UTF-16 code unit, advancing `position` past the bytes it used.
fn read_unit(bytes: &[u8], position: &mut usize) -> Result<u32> {
    let lead = u32::from(*bytes.get(*position).ok_or(Error::InvalidModifiedUtf8)?);
    let width = match lead >> 4 {
        0x0..=0x7 => 1,
        0xC | 0xD => 2,
        0xE => 3,
        _ => return Err(Error::InvalidModifiedUtf8),
    };
    let mut value = match width {
        1 => lead,
        2 => lead & 0x1F,
        _ => lead & 0x0F,
    };
    for offset in 1..width {
        let byte = u32::from(
            *bytes
                .get(*position + offset)
                .ok_or(Error::InvalidModifiedUtf8)?,
        );
        if byte & 0xC0 != 0x80 {
            return Err(Error::InvalidModifiedUtf8);
        }
        value = (value << 6) | (byte & 0x3F);
    }
    *position += width;
    Ok(value)
}

fn encoded_length(value: &str) -> usize {
    value
        .chars()
        .map(|character| match u32::from(character) {
            0 | 0x0080..=0x07FF => 2,
            0x0001..=0x007F => 1,
            0x0800..=0xFFFF => 3,
            // Two surrogates, three bytes each.
            _ => 6,
        })
        .sum()
}

fn write_two(writer: &mut Writer, code_point: u32) {
    writer.u8(0xC0 | low_byte(code_point >> 6));
    writer.u8(0x80 | low_byte(code_point & 0x3F));
}

fn write_three(writer: &mut Writer, code_point: u32) {
    writer.u8(0xE0 | low_byte(code_point >> 12));
    writer.u8(0x80 | low_byte((code_point >> 6) & 0x3F));
    writer.u8(0x80 | low_byte(code_point & 0x3F));
}

/// Byte zero of `value`, which every caller has already masked to fit.
const fn low_byte(value: u32) -> u8 {
    (value & 0xFF).to_le_bytes()[0]
}
