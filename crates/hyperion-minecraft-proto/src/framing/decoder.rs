//! The inbound half of the pipeline.

use super::{
    Cipher, Error, MAX_FRAME_LENGTH, Result, SHARED_SECRET_LEN, compression::Decompressor,
};
use crate::{Decode, Reader};

/// The three bytes `Varint21FrameDecoder` will read before giving up.
const MAX_LENGTH_PREFIX_BYTES: usize = 3;

/// Compact the receive buffer once this many bytes at the front have been
/// consumed. Purely an allocation-churn knob: correctness does not depend on
/// it, and 8 KiB is roughly the point at which the memmove costs less than
/// carrying the dead prefix through another read.
const COMPACT_THRESHOLD: usize = 8192;

/// One decoded packet, borrowed from the decoder's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet<'a> {
    /// The packet id, which names the packet within the current state and
    /// direction. See [`crate::generated::packet_id`].
    pub id: i32,
    /// The body, with the id already stripped.
    pub body: &'a [u8],
}

impl Packet<'_> {
    /// Decode the body as `T`, insisting it consumes every byte.
    ///
    /// The trailing-bytes check is `PacketDecoder`'s: it throws when a packet
    /// leaves anything unread, because that means the layout is wrong rather
    /// than that the packet had extra data.
    ///
    /// # Errors
    /// Whatever `T`'s codec reports, or [`crate::Error::TrailingBytes`].
    pub fn decode_body<'a, T: Decode<'a>>(&'a self) -> Result<T> {
        let mut reader = Reader::new(self.body);
        let value = T::decode(&mut reader)?;
        reader.finish()?;
        Ok(value)
    }
}

/// Turns wire bytes back into packet bodies.
///
/// One per connection, per direction. Feed it whatever a read returned with
/// [`queue`](Self::queue) and then drain whole packets with
/// [`next_packet`](Self::next_packet) until it yields `None`.
#[derive(Debug)]
pub struct FrameDecoder {
    /// Received bytes, already deciphered, from `cursor` onwards unread.
    buffer: Vec<u8>,
    cursor: usize,
    threshold: Option<usize>,
    decompressor: Decompressor,
    cipher: Option<Cipher>,
    /// The body of the packet most recently returned. Compressed frames
    /// inflate into the decompressor's own scratch; this holds the copy for
    /// uncompressed ones so that both cases can borrow from the decoder for
    /// the same lifetime.
    frame: Vec<u8>,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// A decoder with neither compression nor encryption.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            cursor: 0,
            threshold: None,
            decompressor: Decompressor::new(),
            cipher: None,
            frame: Vec::new(),
        }
    }

    /// Start expecting compressed frames at `threshold` bytes, or stop.
    pub const fn set_compression_threshold(&mut self, threshold: Option<usize>) {
        self.threshold = threshold;
    }

    /// The threshold in force, if any.
    #[must_use]
    pub const fn compression_threshold(&self) -> Option<usize> {
        self.threshold
    }

    /// Start deciphering with `secret`, the 16 bytes from `login_key`.
    ///
    /// Any bytes already queued stay as they are, which is correct: the peer
    /// switches ciphers at a frame boundary and anything received before the
    /// switch was sent in the clear.
    pub fn enable_encryption(&mut self, secret: &[u8; SHARED_SECRET_LEN]) {
        self.cipher = Some(Cipher::decryptor(secret));
    }

    /// True once [`enable_encryption`](Self::enable_encryption) has been called.
    #[must_use]
    pub const fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    /// Bytes received but not yet formed into a whole packet.
    #[must_use]
    pub const fn buffered_len(&self) -> usize {
        self.buffer.len() - self.cursor
    }

    /// Hand the decoder bytes straight off the socket.
    ///
    /// They are deciphered here rather than when a frame is assembled: CFB8 is
    /// a stream cipher, so its shift register has to advance over the bytes in
    /// arrival order whether or not they complete a frame yet.
    pub fn queue(&mut self, bytes: &[u8]) {
        let start = self.buffer.len();
        self.buffer.extend_from_slice(bytes);
        if let Some(cipher) = self.cipher.as_mut() {
            cipher.apply(&mut self.buffer[start..]);
        }
    }

    /// Take the next whole packet, or `None` if one has not arrived yet.
    ///
    /// # Errors
    /// Returns a [`Error`] describing the violation for a frame that is
    /// malformed rather than merely incomplete. A connection cannot recover
    /// from any of them, because the stream position is no longer known.
    pub fn next_packet(&mut self) -> Result<Option<Packet<'_>>> {
        let Some((header_len, frame_len)) = self.peek_frame_length()? else {
            return Ok(None);
        };

        let start = self.cursor + header_len;
        let end = start + frame_len;
        if self.buffer.len() < end {
            return Ok(None);
        }

        // The frame is lifted out of the receive buffer before the buffer is
        // compacted, because compacting moves the bytes a borrow would point
        // at. It also gives the two branches below a common lifetime: an
        // inflated body lives in the decompressor and a plain one here, and a
        // single return type needs both to be reachable from `self`.
        self.frame.clear();
        self.frame.extend_from_slice(&self.buffer[start..end]);
        self.cursor = end;
        self.compact();

        let payload = match self.threshold {
            Some(threshold) => Self::decompress(&mut self.decompressor, threshold, &self.frame)?,
            None => &self.frame,
        };

        let mut reader = Reader::new(payload);
        let id = reader.var_int()?;
        let consumed = payload.len() - reader.remaining_len();
        Ok(Some(Packet {
            id,
            body: &payload[consumed..],
        }))
    }

    /// Read the frame header without consuming it.
    ///
    /// Returns the header's own length and the frame length it declares, or
    /// `None` when fewer than a whole header has arrived.
    fn peek_frame_length(&self) -> Result<Option<(usize, usize)>> {
        let unread = &self.buffer[self.cursor..];
        let mut length: i32 = 0;
        for (index, byte) in unread.iter().take(MAX_LENGTH_PREFIX_BYTES).enumerate() {
            length |= i32::from(byte & 0x7F) << (index * 7);
            if byte & 0x80 == 0 {
                if length == 0 {
                    return Err(Error::EmptyFrame);
                }
                let length = usize::try_from(length).map_err(|_| Error::MalformedHeader)?;
                if length > MAX_FRAME_LENGTH {
                    return Err(Error::FrameTooLarge { length });
                }
                return Ok(Some((index + 1, length)));
            }
        }
        // Three continuation bits in a row is the case
        // `Varint21FrameDecoder.copyVarint` throws on: a fourth byte could
        // only describe a frame wider than the 21 bits the splitter allows.
        if unread.len() >= MAX_LENGTH_PREFIX_BYTES {
            return Err(Error::FrameLengthTooWide);
        }
        Ok(None)
    }

    fn decompress<'a>(
        decompressor: &'a mut Decompressor,
        threshold: usize,
        frame: &'a [u8],
    ) -> Result<&'a [u8]> {
        let mut reader = Reader::new(frame);
        let declared = reader.var_int()?;
        let declared = usize::try_from(declared).map_err(|_| Error::MalformedHeader)?;
        let payload = reader.remaining();

        // Zero is the "sent uncompressed" marker rather than a length, so it
        // is checked before the range validation below, which would otherwise
        // reject every small packet.
        if declared == 0 {
            return Ok(payload);
        }

        // `CompressionDecoder` gates both of these on `validateDecompressed`,
        // which a server sets for a client connection and a client clears for
        // the server's. This decoder always validates: it is the strict side
        // of the pair, and a peer that trips either check has sent something
        // its own encoder would not have produced.
        if declared < threshold {
            return Err(Error::BadlyCompressed {
                declared,
                threshold,
            });
        }
        if declared > super::MAX_UNCOMPRESSED_LENGTH {
            return Err(Error::PacketTooLarge { length: declared });
        }
        decompressor.inflate(declared, payload)
    }

    /// Drop consumed bytes off the front of the buffer once enough have piled
    /// up to be worth the move.
    fn compact(&mut self) {
        if self.cursor == self.buffer.len() {
            self.buffer.clear();
            self.cursor = 0;
        } else if self.cursor >= COMPACT_THRESHOLD {
            self.buffer.drain(..self.cursor);
            self.cursor = 0;
        }
    }
}
