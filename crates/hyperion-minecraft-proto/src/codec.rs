//! Wire primitives.
//!
//! Each function mirrors the corresponding method in the server's own
//! `net.minecraft.network` classes, so limits and error cases match the server
//! rather than approximating it.

use crate::{Error, Result};

/// Maximum bytes a `VarInt` may occupy (`VarInt.MAX_VARINT_SIZE`).
pub const MAX_VARINT_SIZE: usize = 5;

/// Maximum bytes a `VarLong` may occupy.
pub const MAX_VARLONG_SIZE: usize = 10;

/// Default string limit (`FriendlyByteBuf.MAX_STRING_LENGTH`, `Short.MAX_VALUE`).
pub const MAX_STRING_LENGTH: usize = 32767;

/// Reads values out of a byte slice, tracking how far it has advanced.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    /// Start reading at the beginning of `data`.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Bytes not yet consumed.
    #[must_use]
    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }

    /// Number of bytes not yet consumed.
    #[must_use]
    pub const fn remaining_len(&self) -> usize {
        self.data.len() - self.position
    }

    /// True once every byte has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.remaining_len() == 0
    }

    /// Fail unless the reader is exhausted.
    ///
    /// Packet bodies are length-delimited by the frame, so anything left over
    /// means the layout is wrong. Catching that here turns a silent
    /// misinterpretation into an error at the point it happens.
    ///
    /// # Errors
    /// Returns [`Error::TrailingBytes`] when bytes remain.
    pub const fn finish(&self) -> Result<()> {
        if self.remaining_len() == 0 {
            Ok(())
        } else {
            Err(Error::TrailingBytes(self.remaining_len()))
        }
    }

    /// Take exactly `count` bytes.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] when fewer than `count` bytes remain.
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.remaining_len() < count {
            return Err(Error::UnexpectedEof {
                needed: count,
                available: self.remaining_len(),
            });
        }
        let out = &self.data[self.position..self.position + count];
        self.position += count;
        Ok(out)
    }

    /// Read a single byte.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Read a boolean, encoded as one byte.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    /// Read a signed byte.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn i8(&mut self) -> Result<i8> {
        Ok(i8::from_ne_bytes([self.u8()?]))
    }

    /// Read a big-endian unsigned short.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Read a big-endian signed short.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_ne_bytes(self.u16()?.to_ne_bytes()))
    }

    /// Read a big-endian signed int.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn i32(&mut self) -> Result<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a big-endian signed long.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn i64(&mut self) -> Result<i64> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Read a big-endian `f32`.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn f32(&mut self) -> Result<f32> {
        let bytes = self.take(4)?;
        Ok(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a big-endian `f64`.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn f64(&mut self) -> Result<f64> {
        let bytes = self.take(8)?;
        Ok(f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Read a `VarInt`.
    ///
    /// Mirrors `VarInt.read`: seven data bits per byte, little-endian groups,
    /// at most five bytes.
    ///
    /// # Errors
    /// Returns [`Error::VarIntTooLong`] past five bytes, or
    /// [`Error::UnexpectedEof`] on truncated input.
    pub fn var_int(&mut self) -> Result<i32> {
        let mut out: i32 = 0;
        for index in 0..MAX_VARINT_SIZE {
            let byte = self.u8()?;
            out |= i32::from(byte & 0x7F) << (index * 7);
            if byte & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(Error::VarIntTooLong)
    }

    /// Read a `VarLong`.
    ///
    /// # Errors
    /// Returns [`Error::VarIntTooLong`] past ten bytes, or
    /// [`Error::UnexpectedEof`] on truncated input.
    pub fn var_long(&mut self) -> Result<i64> {
        let mut out: i64 = 0;
        for index in 0..MAX_VARLONG_SIZE {
            let byte = self.u8()?;
            out |= i64::from(byte & 0x7F) << (index * 7);
            if byte & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(Error::VarIntTooLong)
    }

    /// Read a length-prefixed byte array.
    ///
    /// # Errors
    /// Returns an error on a negative length or truncated input.
    pub fn byte_array(&mut self) -> Result<&'a [u8]> {
        let length = self.var_int()?;
        let length = usize::try_from(length).map_err(|_| Error::NegativeLength(length))?;
        self.take(length)
    }

    /// Read a UTF-8 string limited to [`MAX_STRING_LENGTH`] characters.
    ///
    /// # Errors
    /// See [`Reader::string_with_limit`].
    pub fn string(&mut self) -> Result<&'a str> {
        self.string_with_limit(MAX_STRING_LENGTH)
    }

    /// Read a UTF-8 string limited to `max` characters.
    ///
    /// Mirrors `Utf8String.read`: the prefix counts *bytes*, the limit counts
    /// UTF-16 code units the way Java's `String.length()` does, and the byte
    /// budget is three times the character limit.
    ///
    /// # Errors
    /// Returns [`Error::StringTooLong`] when either limit is exceeded,
    /// [`Error::InvalidUtf8`] on malformed bytes, or [`Error::UnexpectedEof`]
    /// on truncated input.
    pub fn string_with_limit(&mut self, max: usize) -> Result<&'a str> {
        let byte_length = self.var_int()?;
        let byte_length =
            usize::try_from(byte_length).map_err(|_| Error::NegativeLength(byte_length))?;
        let max_bytes = max * 3;
        if byte_length > max_bytes {
            return Err(Error::StringTooLong {
                length: byte_length,
                max: max_bytes,
            });
        }
        let bytes = self.take(byte_length)?;
        let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
        let utf16_len = text.encode_utf16().count();
        if utf16_len > max {
            return Err(Error::StringTooLong {
                length: utf16_len,
                max,
            });
        }
        Ok(text)
    }

    /// Read a UUID as two big-endian longs, most significant first.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] at end of input.
    pub fn uuid(&mut self) -> Result<u128> {
        let bytes = self.take(16)?;
        let mut array = [0u8; 16];
        array.copy_from_slice(bytes);
        Ok(u128::from_be_bytes(array))
    }
}

/// Appends values to a growable buffer.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buffer: Vec<u8>,
}

impl Writer {
    /// A writer over an empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// The bytes written so far.
    // Not const: deref coercion from Vec to a slice is not available in const
    // context. clippy used to suggest it anyway; nightly-2025-05-05 no longer
    // does, so the expect attribute that silenced it is gone.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    /// Consume the writer, yielding its buffer.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.buffer
    }

    /// Append raw bytes.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Append a single byte.
    pub fn u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    /// Append a boolean as one byte.
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// Append a signed byte.
    pub fn i8(&mut self, value: i8) {
        self.u8(value.to_ne_bytes()[0]);
    }

    /// Append a big-endian unsigned short.
    pub fn u16(&mut self, value: u16) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a big-endian signed short.
    pub fn i16(&mut self, value: i16) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a big-endian signed int.
    pub fn i32(&mut self, value: i32) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a big-endian signed long.
    pub fn i64(&mut self, value: i64) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a big-endian `f32`.
    pub fn f32(&mut self, value: f32) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a big-endian `f64`.
    pub fn f64(&mut self, value: f64) {
        self.raw(&value.to_be_bytes());
    }

    /// Append a `VarInt`, mirroring `VarInt.write`.
    ///
    /// Negative values are written as their unsigned bit pattern and always
    /// occupy the full five bytes, which is why the server treats a `VarInt`
    /// field as signed but frames it as unsigned.
    pub fn var_int(&mut self, value: i32) {
        // Reinterpreting the bits rather than casting: the shift below must be
        // logical, and a signed right-shift would loop forever on a negative.
        let mut remaining = u32::from_ne_bytes(value.to_ne_bytes());
        loop {
            // The low seven bits always fit in a byte, so taking byte zero of
            // the little-endian representation is exact, not a truncation.
            let low = (remaining & 0x7F).to_le_bytes()[0];
            if remaining & !0x7F == 0 {
                self.u8(low);
                return;
            }
            self.u8(low | 0x80);
            remaining >>= 7;
        }
    }

    /// Append a `VarLong`.
    pub fn var_long(&mut self, value: i64) {
        let mut remaining = u64::from_ne_bytes(value.to_ne_bytes());
        loop {
            let low = (remaining & 0x7F).to_le_bytes()[0];
            if remaining & !0x7F == 0 {
                self.u8(low);
                return;
            }
            self.u8(low | 0x80);
            remaining >>= 7;
        }
    }

    /// Append a length-prefixed byte array.
    ///
    /// # Errors
    /// Returns [`Error::NegativeLength`] if the slice is longer than a `VarInt`
    /// can describe, which no real packet reaches but which would otherwise
    /// write a corrupt length.
    pub fn byte_array(&mut self, bytes: &[u8]) -> Result<()> {
        let length = i32::try_from(bytes.len()).map_err(|_| Error::NegativeLength(-1))?;
        self.var_int(length);
        self.raw(bytes);
        Ok(())
    }

    /// Append a string limited to [`MAX_STRING_LENGTH`] characters.
    ///
    /// # Errors
    /// See [`Writer::string_with_limit`].
    pub fn string(&mut self, value: &str) -> Result<()> {
        self.string_with_limit(value, MAX_STRING_LENGTH)
    }

    /// Append a string limited to `max` characters.
    ///
    /// # Errors
    /// Returns [`Error::StringTooLong`] when the value exceeds `max` UTF-16
    /// code units, matching the server's own check in `Utf8String.write`.
    pub fn string_with_limit(&mut self, value: &str, max: usize) -> Result<()> {
        let utf16_len = value.encode_utf16().count();
        if utf16_len > max {
            return Err(Error::StringTooLong {
                length: utf16_len,
                max,
            });
        }
        self.byte_array(value.as_bytes())
    }

    /// Append a UUID as two big-endian longs, most significant first.
    pub fn uuid(&mut self, value: u128) {
        self.raw(&value.to_be_bytes());
    }
}
