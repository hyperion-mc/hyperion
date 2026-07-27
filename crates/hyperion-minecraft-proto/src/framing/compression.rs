//! The deflate layer, and the convention that lets it be skipped per packet.
//!
//! Once `login_compression` names a threshold, every frame gains a second
//! `VarInt` header. It holds the uncompressed length, and `0` is reserved to
//! mean "not compressed" -- `CompressionEncoder.encode` writes it for any
//! packet shorter than the threshold and `CompressionDecoder.decode` returns
//! the remainder verbatim when it reads one.
//!
//! The comparison in the encoder is strict:
//!
//! ```java
//! if (uncompressedLength < this.threshold) { VarInt.write(out, 0); ... }
//! ```
//!
//! so a packet of exactly `threshold` bytes *is* compressed. The wiki has
//! historically described this as "greater than the threshold"; the source
//! says at least.

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

use super::{Error, Result, write_var_int};

/// Largest packet the compression layer will carry
/// (`CompressionDecoder.MAXIMUM_UNCOMPRESSED_LENGTH`, and the same limit
/// spelled `0x800000` inline in `CompressionEncoder.encode`).
pub const MAX_UNCOMPRESSED_LENGTH: usize = 0x0080_0000;

/// `CompressionDecoder.MAXIMUM_COMPRESSED_LENGTH`.
///
/// Declared but never read in 26.2: the field is initialised and nothing in
/// `decode` consults it, because `Varint21FrameDecoder` has already capped a
/// frame at 21 bits, which is half a byte short of this number. It is exposed
/// here for completeness rather than enforced twice.
pub const MAX_COMPRESSED_LENGTH: usize = 0x0020_0000;

/// zlib deflate at the level `java.util.zip.Deflater`'s no-argument
/// constructor picks (`Deflater.DEFAULT_COMPRESSION`, which is level 6).
///
/// The level only changes how many bytes come out, never whether the peer can
/// read them, so this matching the server is a bandwidth question rather than
/// a correctness one.
const DEFAULT_LEVEL: Compression = Compression::new(6);

/// The outbound half of the compression layer.
#[derive(Debug)]
pub(super) struct Compressor {
    /// Reused across packets exactly as `CompressionEncoder` reuses its
    /// `Deflater`, so a connection allocates one zlib window rather than one
    /// per packet.
    deflate: Compress,
}

impl Compressor {
    pub(super) fn new() -> Self {
        Self {
            deflate: Compress::new(DEFAULT_LEVEL, true),
        }
    }

    /// Write the compressed form of `body` to `out`, including the
    /// uncompressed-length header.
    ///
    /// `threshold` is the value from `login_compression`.
    pub(super) fn write(&mut self, threshold: usize, body: &[u8], out: &mut Vec<u8>) -> Result<()> {
        if body.len() > MAX_UNCOMPRESSED_LENGTH {
            return Err(Error::PacketTooLarge { length: body.len() });
        }

        if body.len() < threshold {
            write_var_int(out, 0);
            out.extend_from_slice(body);
            return Ok(());
        }

        let length =
            i32::try_from(body.len()).map_err(|_| Error::PacketTooLarge { length: body.len() })?;
        write_var_int(out, length);

        self.deflate.reset();
        // A deflate stream is never longer than its input by more than the
        // block overhead, so this is a ceiling rather than a guess and the
        // loop below tops it up if zlib ever disagrees.
        let mut scratch = vec![0u8; body.len() + 64];
        let mut consumed = 0usize;
        loop {
            let before_in = self.deflate.total_in();
            let before_out = self.deflate.total_out();
            let status = self
                .deflate
                .compress(&body[consumed..], &mut scratch, FlushCompress::Finish)
                .map_err(|error| Error::Inflate(error.to_string()))?;
            let produced = usize::try_from(self.deflate.total_out() - before_out)
                .expect("deflate output fits in a usize");
            consumed += usize::try_from(self.deflate.total_in() - before_in)
                .expect("deflate input fits in a usize");
            out.extend_from_slice(&scratch[..produced]);
            match status {
                Status::StreamEnd => return Ok(()),
                // BufError with nothing produced would spin; it cannot happen
                // with a buffer this size, but treating it as an error is
                // cheaper than trusting that forever.
                Status::Ok => {}
                Status::BufError if produced > 0 => {}
                Status::BufError => {
                    return Err(Error::Inflate("deflate made no progress".to_owned()));
                }
            }
        }
    }
}

/// The inbound half of the compression layer.
#[derive(Debug)]
pub(super) struct Decompressor {
    inflate: Decompress,
    /// Inflated bytes for the frame currently being read.
    scratch: Vec<u8>,
}

impl Decompressor {
    pub(super) fn new() -> Self {
        Self {
            inflate: Decompress::new(true),
            scratch: Vec::new(),
        }
    }

    /// Inflate `payload` to exactly `declared` bytes.
    ///
    /// `CompressionDecoder.inflate` allocates a buffer of the declared size
    /// and then insists the inflater filled it exactly, which is what stops a
    /// zip bomb and what catches a truncated stream. Both checks are kept.
    pub(super) fn inflate(&mut self, declared: usize, payload: &[u8]) -> Result<&[u8]> {
        self.scratch.clear();
        self.scratch.reserve(declared);
        self.inflate.reset(true);

        // `reserve` guarantees the capacity; writing through the spare
        // capacity would need unsafe, so the buffer is zeroed instead. The
        // cost is one memset of at most 8 MiB per compressed packet.
        self.scratch.resize(declared, 0);
        let status = self
            .inflate
            .decompress(payload, &mut self.scratch, FlushDecompress::Finish)
            .map_err(|error| Error::Inflate(error.to_string()))?;
        let produced =
            usize::try_from(self.inflate.total_out()).expect("inflate output fits in a usize");

        // A stream that has not ended has more to give than the frame said it
        // would, which is the zip-bomb case; one that ended short is a
        // truncation. `CompressionDecoder` reports both as the same mismatch.
        if produced != declared || status != Status::StreamEnd {
            return Err(Error::LengthMismatch {
                declared,
                actual: produced,
            });
        }
        Ok(&self.scratch)
    }
}
