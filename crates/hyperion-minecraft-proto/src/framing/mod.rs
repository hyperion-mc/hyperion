//! Putting packets on a socket and taking them off again.
//!
//! Everything under [`crate::packets`] describes a packet *body*. This module
//! is the rest: the length prefix, the packet-id prefix, the optional deflate
//! layer and the optional cipher. Together they are the four netty handlers
//! `Connection.configureSerialization` installs, in the order it installs
//! them.
//!
//! # The pipeline, and why the order is what it is
//!
//! `Connection.configureSerialization` builds, head to tail:
//!
//! ```text
//! splitter (Varint21FrameDecoder) -> decoder (PacketDecoder)
//!                                 -> prepender (Varint21LengthFieldPrepender)
//!                                 -> encoder (PacketEncoder)
//! ```
//!
//! `Connection.setupCompression` then splices `decompress` *after* `splitter`
//! and `compress` *after* `prepender`, and `Connection.setEncryptionKey`
//! splices `decrypt` *before* `splitter` and `encrypt` *before* `prepender`.
//! Netty runs inbound handlers head to tail and outbound handlers tail to
//! head, so the effective orders are:
//!
//! ```text
//! inbound:  decrypt -> split frames -> decompress -> read id, read body
//! outbound: write id, write body -> compress -> prepend length -> encrypt
//! ```
//!
//! The consequence worth remembering is that the length prefix counts
//! *compressed* bytes and is itself *encrypted*. A decoder therefore cannot
//! read the length until it has decrypted, which is why [`FrameDecoder`]
//! deciphers on arrival rather than per frame.
//!
//! # Use
//!
//! ```
//! use hyperion_minecraft_proto::framing::{FrameDecoder, FrameEncoder};
//!
//! let mut encoder = FrameEncoder::new();
//! let mut wire = Vec::new();
//! encoder.encode(0x00, b"body bytes", &mut wire)?;
//!
//! let mut decoder = FrameDecoder::new();
//! decoder.queue(&wire);
//! let packet = decoder.next_packet()?.expect("one whole frame was queued");
//! assert_eq!(packet.id, 0x00);
//! assert_eq!(packet.body, b"body bytes");
//! # Ok::<(), hyperion_minecraft_proto::framing::Error>(())
//! ```
//!
//! Both halves start out plain. [`FrameEncoder::set_compression_threshold`]
//! and [`FrameDecoder::set_compression_threshold`] turn on deflate at the
//! point `login_compression` is sent or received;
//! [`FrameEncoder::enable_encryption`] and
//! [`FrameDecoder::enable_encryption`] turn on the cipher at the point the
//! shared secret is agreed. Neither can be turned off again, matching the
//! server: `setEncryptionKey` only ever adds handlers.

mod compression;
mod decoder;
mod encoder;
mod encryption;

use std::fmt;

pub use compression::{MAX_COMPRESSED_LENGTH, MAX_UNCOMPRESSED_LENGTH};
pub use decoder::{FrameDecoder, Packet};
pub use encoder::FrameEncoder;
pub use encryption::{Cipher, SHARED_SECRET_LEN};

/// The widest frame `Varint21FrameDecoder` will accept.
///
/// It reads at most three bytes of length prefix and rejects anything longer
/// with `CorruptedFrameException("length wider than 21-bit")`, so 21 bits is
/// a hard ceiling on a frame regardless of what the compression layer allows.
pub const MAX_FRAME_LENGTH: usize = (1 << 21) - 1;

/// Result alias for framing operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Something went wrong framing or unframing a packet.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A frame's length prefix ran past three bytes.
    ///
    /// `Varint21FrameDecoder.copyVarint` throws rather than reading a fourth
    /// byte, so this is a corrupt stream and not a large packet.
    FrameLengthTooWide,
    /// A frame declared a length of zero.
    ///
    /// `Varint21FrameDecoder` rejects it explicitly: every frame carries at
    /// least a packet id.
    EmptyFrame,
    /// A frame was longer than [`MAX_FRAME_LENGTH`].
    FrameTooLarge {
        /// Length the frame declared or would have needed.
        length: usize,
    },
    /// A packet body exceeded [`MAX_UNCOMPRESSED_LENGTH`].
    ///
    /// `CompressionEncoder.encode` throws `IllegalArgumentException` here, and
    /// `CompressionDecoder` refuses the mirror case on the way in.
    PacketTooLarge {
        /// Uncompressed length seen.
        length: usize,
    },
    /// A compressed frame declared an uncompressed length below the
    /// negotiated threshold.
    ///
    /// The sender should have left it uncompressed, so accepting it would let
    /// a peer choose which of two encodings to use for the same bytes.
    /// `CompressionDecoder` rejects it whenever `validateDecompressed` is set,
    /// which is what a server does for a client connection.
    BadlyCompressed {
        /// Length the frame declared.
        declared: usize,
        /// Threshold in force.
        threshold: usize,
    },
    /// Inflating a frame produced a different number of bytes than declared.
    LengthMismatch {
        /// Length the frame declared.
        declared: usize,
        /// Bytes actually produced.
        actual: usize,
    },
    /// zlib rejected the compressed payload.
    Inflate(String),
    /// A `VarInt` in a frame header ran past its limit or its input.
    MalformedHeader,
    /// Decoding the packet id or body failed.
    Codec(crate::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameLengthTooWide => f.write_str("frame length prefix wider than 21 bits"),
            Self::EmptyFrame => f.write_str("frame length was zero"),
            Self::FrameTooLarge { length } => {
                write!(
                    f,
                    "frame of {length} bytes exceeds the {MAX_FRAME_LENGTH}-byte maximum"
                )
            }
            Self::PacketTooLarge { length } => write!(
                f,
                "packet of {length} bytes exceeds the {MAX_UNCOMPRESSED_LENGTH}-byte maximum"
            ),
            Self::BadlyCompressed {
                declared,
                threshold,
            } => write!(
                f,
                "compressed frame declared {declared} bytes, below the {threshold}-byte threshold"
            ),
            Self::LengthMismatch { declared, actual } => write!(
                f,
                "frame declared {declared} uncompressed bytes but inflated to {actual}"
            ),
            Self::Inflate(message) => write!(f, "could not inflate frame: {message}"),
            Self::MalformedHeader => f.write_str("malformed frame header"),
            Self::Codec(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<crate::Error> for Error {
    fn from(error: crate::Error) -> Self {
        Self::Codec(error)
    }
}

/// Append a `VarInt` to a byte buffer.
///
/// [`Writer::var_int`](crate::Writer::var_int) does the same thing, but the
/// framing layer works in raw buffers it can `clear` and reuse, and a `Writer`
/// has no way to be emptied without dropping its allocation.
pub(crate) fn write_var_int(out: &mut Vec<u8>, value: i32) {
    // The bit pattern, not the value: the shift has to be logical or a
    // negative id would loop forever.
    let mut remaining = u32::from_ne_bytes(value.to_ne_bytes());
    loop {
        let low = (remaining & 0x7F).to_le_bytes()[0];
        if remaining & !0x7F == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
        remaining >>= 7;
    }
}
