#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "todo"
)]

//! Encoding of packets.

use std::{
    fmt::Debug,
    io::{Cursor, Write},
};

use anyhow::ensure;
use tracing::trace;
use valence_protocol::{CompressionThreshold, Encode, VarInt};

use crate::{PacketBundle, ScratchBuffer, net::MAX_PACKET_SIZE, storage::Buf};

mod util;

/// A struct which represents a particular encoding method.
#[derive(Default)]
pub struct PacketEncoder {
    threshold: CompressionThreshold,
}

impl Debug for PacketEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacketEncoder")
            .field("threshold", &self.threshold)
            .finish()
    }
}

/// Append a packet to the buffer without compression.
pub fn append_packet_without_compression<P, B: Buf>(
    pkt: P,
    buf: &mut B,
) -> anyhow::Result<B::Output>
where
    P: PacketBundle,
{
    let data_write_start = VarInt::MAX_SIZE as u64;
    let slice = buf.get_contiguous(MAX_PACKET_SIZE);

    let mut cursor = Cursor::new(slice);
    cursor.set_position(data_write_start);

    pkt.encode_including_ids(&mut cursor)?;

    let data_len = usize::try_from(cursor.position())? - usize::try_from(data_write_start)?;

    let packet_len_size = VarInt(i32::try_from(data_len)?).written_size();

    let packet_len = packet_len_size + data_len;
    ensure!(
        packet_len <= MAX_PACKET_SIZE,
        "packet exceeds maximum length"
    );

    let inner = cursor.into_inner();

    inner.copy_within(
        usize::try_from(data_write_start)?..usize::try_from(data_write_start)? + data_len,
        packet_len_size,
    );

    let mut cursor = Cursor::new(inner);
    VarInt(i32::try_from(data_len)?).encode(&mut cursor)?;

    let slice = cursor.into_inner();

    #[expect(
        clippy::indexing_slicing,
        reason = "this is probably fine? todo: verify"
    )]
    let entire_slice = &slice[..packet_len_size + data_len];

    let len = entire_slice.len();

    trace!("without compression: {len} bytes");

    Ok(buf.advance(len))
}

impl PacketEncoder {
    /// Creates a new [`PacketEncoder`] with the given compression threshold.
    #[must_use]
    pub const fn new(threshold: CompressionThreshold) -> Self {
        Self { threshold }
    }

    /// Obtains the compression threshold.
    #[must_use]
    pub const fn compression_threshold(&self) -> CompressionThreshold {
        self.threshold
    }

    /// Appends a packet to the buffer with compression.
    pub fn append_packet_with_compression<P, B: Buf>(
        &self,
        packet: P,
        buf: &mut B,
        scratch: &mut impl ScratchBuffer,
        compressor: &mut libdeflater::Compressor,
    ) -> anyhow::Result<B::Output>
    where
        P: PacketBundle,
    {
        const DATA_LEN_0_SIZE: usize = 1;

        // + 1 because data len would be 0 if not compressed
        let data_write_start = (VarInt::MAX_SIZE + DATA_LEN_0_SIZE) as u64;
        let slice = buf.get_contiguous(MAX_PACKET_SIZE);

        let mut cursor = Cursor::new(&mut slice[..]);
        cursor.set_position(data_write_start);

        packet.encode_including_ids(&mut cursor)?;

        let end_data_position_exclusive = cursor.position();

        let data_len = end_data_position_exclusive - data_write_start;

        let threshold = u64::from(self.threshold.0.unsigned_abs());

        if data_len > threshold {
            let scratch = scratch.obtain();

            let data_slice = &mut slice
                [usize::try_from(data_write_start)?..usize::try_from(end_data_position_exclusive)?];

            let written = {
                let len = data_slice.len();
                let span = tracing::trace_span!("zlib_compress", bytes = len);
                let _enter = span.enter();
                compressor.zlib_compress(data_slice, scratch).unwrap()
            };

            let compressed = &scratch[..written];

            let data_len = VarInt(data_len as u32 as i32);

            let packet_len = data_len.written_size() + compressed.len();
            let packet_len = VarInt(packet_len as u32 as i32);

            let mut write = Cursor::new(&mut slice[..]);
            packet_len.encode(&mut write)?;
            data_len.encode(&mut write)?;
            write.write_all(compressed)?;

            let len = write.position();

            return Ok(buf.advance(len as usize));
        }

        let data_len_0 = VarInt(0);
        let packet_len = VarInt(DATA_LEN_0_SIZE as i32 + data_len as u32 as i32); // packet_len.written_size();

        let mut cursor = Cursor::new(&mut slice[..]);
        packet_len.encode(&mut cursor)?;
        data_len_0.encode(&mut cursor)?;

        let pos = cursor.position();

        slice.copy_within(
            data_write_start as usize..end_data_position_exclusive as usize,
            pos as usize,
        );

        let len = pos as u32 + (end_data_position_exclusive - data_write_start) as u32;

        Ok(buf.advance(len as usize))
    }

    /// Appends a packet to the buffer which may or may not be compressed.
    pub fn append_packet<P, B: Buf>(
        &self,
        pkt: P,
        buf: &mut B,
        scratch: &mut impl ScratchBuffer,
        compressor: &mut libdeflater::Compressor,
    ) -> anyhow::Result<B::Output>
    where
        P: PacketBundle,
    {
        let has_compression = self.threshold.0 >= 0;

        if has_compression {
            self.append_packet_with_compression(pkt, buf, scratch, compressor)
        } else {
            append_packet_without_compression(pkt, buf)
        }
    }

    /// Sets the compression threshold.
    pub const fn set_compression(&mut self, threshold: CompressionThreshold) {
        self.threshold = threshold;
    }
}
