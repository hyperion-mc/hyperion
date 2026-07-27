//! The outbound half of the pipeline.

use super::{
    Cipher, Error, MAX_FRAME_LENGTH, Result, SHARED_SECRET_LEN, compression::Compressor,
    write_var_int,
};
use crate::{Encode, Writer};

/// Turns packet bodies into wire bytes.
///
/// One per connection, per direction. It carries the deflate window and the
/// cipher's shift register, so the same instance must see every packet a
/// connection sends, in order.
#[derive(Debug)]
pub struct FrameEncoder {
    threshold: Option<usize>,
    compressor: Compressor,
    cipher: Option<Cipher>,
    /// The frame body under construction: packet id and body, then deflated
    /// if compression is on. Reused across packets so a steady-state
    /// connection does not allocate.
    staging: Vec<u8>,
}

impl Default for FrameEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameEncoder {
    /// An encoder with neither compression nor encryption, which is what a
    /// connection starts out as.
    #[must_use]
    pub fn new() -> Self {
        Self {
            threshold: None,
            compressor: Compressor::new(),
            cipher: None,
            staging: Vec::new(),
        }
    }

    /// Start compressing at `threshold` bytes, or stop compressing entirely.
    ///
    /// `Connection.setupCompression` removes both handlers for a negative
    /// threshold, which is how vanilla spells "off"; `None` here is that case.
    pub const fn set_compression_threshold(&mut self, threshold: Option<usize>) {
        self.threshold = threshold;
    }

    /// The threshold in force, if any.
    #[must_use]
    pub const fn compression_threshold(&self) -> Option<usize> {
        self.threshold
    }

    /// Start enciphering with `secret`, the 16 bytes from `login_key`.
    ///
    /// There is no way back: `Connection.setEncryptionKey` only ever adds
    /// handlers, so a connection is plaintext or encrypted for its whole life.
    pub fn enable_encryption(&mut self, secret: &[u8; SHARED_SECRET_LEN]) {
        self.cipher = Some(Cipher::encryptor(secret));
    }

    /// True once [`enable_encryption`](Self::enable_encryption) has been called.
    #[must_use]
    pub const fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    /// Append one framed packet to `out`.
    ///
    /// `body` is the packet body only: the id is prefixed here, because it
    /// sits inside the compressed and length-counted region.
    ///
    /// # Errors
    /// Returns [`Error::PacketTooLarge`] for a body past
    /// [`MAX_UNCOMPRESSED_LENGTH`](super::MAX_UNCOMPRESSED_LENGTH), or
    /// [`Error::FrameTooLarge`] when the framed result will not fit the
    /// 21-bit length prefix.
    pub fn encode(&mut self, packet_id: i32, body: &[u8], out: &mut Vec<u8>) -> Result<()> {
        self.staging.clear();

        match self.threshold {
            Some(threshold) => {
                // The compression header counts the id and the body together:
                // `PacketEncoder` has written both before `CompressionEncoder`
                // sees the buffer.
                let mut plain = Vec::with_capacity(body.len() + 5);
                write_var_int(&mut plain, packet_id);
                plain.extend_from_slice(body);
                self.compressor
                    .write(threshold, &plain, &mut self.staging)?;
            }
            None => {
                write_var_int(&mut self.staging, packet_id);
                self.staging.extend_from_slice(body);
            }
        }

        if self.staging.len() > MAX_FRAME_LENGTH {
            return Err(Error::FrameTooLarge {
                length: self.staging.len(),
            });
        }
        let length = i32::try_from(self.staging.len()).map_err(|_| Error::FrameTooLarge {
            length: self.staging.len(),
        })?;

        let start = out.len();
        write_var_int(out, length);
        out.extend_from_slice(&self.staging);

        // Encryption is the last handler and covers the length prefix too, so
        // it runs over everything this call appended rather than the body only.
        if let Some(cipher) = self.cipher.as_mut() {
            cipher.apply(&mut out[start..]);
        }
        Ok(())
    }

    /// Encode a typed packet, writing its body with its own [`Encode`] impl.
    ///
    /// # Errors
    /// As [`encode`](Self::encode), plus whatever the body's codec reports.
    pub fn encode_packet<T: Encode>(
        &mut self,
        packet_id: i32,
        packet: &T,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let mut body = Writer::new();
        packet.encode(&mut body)?;
        self.encode(packet_id, body.as_slice(), out)
    }
}
