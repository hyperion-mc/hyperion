use std::{
    cell::Cell,
    ops::{Index, RangeFull},
};

use anyhow::{Context, bail, ensure};
use bevy::prelude::*;
use bytes::Buf;
use valence_protocol::{
    CompressionThreshold, Decode, MAX_PACKET_SIZE, Packet, VarInt, var_int::VarIntDecodeError,
};

#[derive(Default)]
struct RefBytesMut {
    cursor: usize,
    inner: Vec<u8>,
}

impl RefBytesMut {
    pub fn advance(&mut self, amount: usize) {
        self.cursor += amount;
    }

    pub fn split_to(&mut self, len: usize) -> &[u8] {
        let before = self.cursor;
        let after = before + len;
        self.cursor = after;

        #[expect(
            clippy::indexing_slicing,
            reason = "this is probably fine? todo: verify"
        )]
        &self.inner[before..after]
    }
}

impl Index<RangeFull> for RefBytesMut {
    type Output = [u8];

    fn index(&self, _: RangeFull) -> &Self::Output {
        let on = self.cursor;
        #[expect(
            clippy::indexing_slicing,
            reason = "this is probably fine? todo: verify"
        )]
        &self.inner[on..]
    }
}

/// A buffer for saving bytes that are not yet decoded.
#[derive(Default, Component)]
pub struct PacketDecoder {
    buf: RefBytesMut,
    threshold: CompressionThreshold,
}

#[derive(Copy, Clone)]
pub struct BorrowedPacketFrame<'a> {
    /// The ID of the decoded packet.
    pub id: i32,
    /// The contents of the packet after the leading [`VarInt`] ID.
    pub body: &'a [u8],
}

impl std::fmt::Debug for BorrowedPacketFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorrowedPacketFrame")
            .field("id", &format!("0x{:x}", self.id))
            .field("body", &bytes::Bytes::copy_from_slice(self.body))
            .finish()
    }
}

impl<'a> BorrowedPacketFrame<'a> {
    /// Attempts to decode this packet as type `P`. An error is returned if the
    /// packet ID does not match, the body of the packet failed to decode, or
    /// some input was missed.
    pub fn decode<P>(&self) -> anyhow::Result<P>
    where
        P: Packet + Decode<'a>,
    {
        ensure!(
            P::ID == self.id,
            "packet ID mismatch while decoding '{}': expected {}, got {}",
            P::NAME,
            P::ID,
            self.id
        );

        let mut r = self.body;

        let pkt = P::decode(&mut r)?;

        ensure!(
            r.is_empty(),
            "missed {} bytes while decoding '{}'",
            r.len(),
            P::NAME
        );

        Ok(pkt)
    }
}

impl PacketDecoder {
    /// Tries to get the next packet from the buffer.
    /// If a new packet is found, the buffer will be truncated by the length of the packet.
    pub fn try_next_packet<'b>(
        &'b mut self,
        bump: &'b bumpalo::Bump,
    ) -> anyhow::Result<Option<BorrowedPacketFrame<'b>>> {
        let mut r = &self.buf[..];

        let packet_len = match VarInt::decode_partial(&mut r) {
            Ok(len) => len,
            Err(VarIntDecodeError::Incomplete) => return Ok(None),
            Err(VarIntDecodeError::TooLarge) => bail!("malformed packet length VarInt"),
        };

        ensure!(
            (0..=MAX_PACKET_SIZE).contains(&packet_len),
            "packet length of {packet_len} is out of bounds"
        );

        #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
        if r.len() < packet_len as usize {
            // Not enough data arrived yet.
            return Ok(None);
        }

        let packet_len_len = VarInt(packet_len).written_size();

        let mut data;

        #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
        if self.threshold.0 >= 0 {
            r = &r[..packet_len as usize];

            let data_len = VarInt::decode(&mut r)?.0;

            ensure!(
                (0..MAX_PACKET_SIZE).contains(&data_len),
                "decompressed packet length of {data_len} is out of bounds"
            );

            // Is this packet compressed?
            if data_len > 0 {
                ensure!(
                    data_len > self.threshold.0,
                    "decompressed packet length of {data_len} is <= the compression threshold of \
                     {}",
                    self.threshold.0
                );

                // todo(perf): make uninit memory ...  MaybeUninit
                let decompression_buf: &mut [u8] = bump.alloc_slice_fill_default(data_len as usize);

                let written_len = {
                    // todo: does it make sense to cache ever?
                    let mut decompressor = libdeflater::Decompressor::new();

                    decompressor.zlib_decompress(r, decompression_buf)?
                };

                debug_assert_eq!(
                    written_len, data_len as usize,
                    "{written_len} != {data_len}"
                );

                let total_packet_len = VarInt(packet_len).written_size() + packet_len as usize;

                self.buf.advance(total_packet_len);

                data = &*decompression_buf;
            } else {
                debug_assert_eq!(data_len, 0, "{data_len} != 0");

                ensure!(
                    r.len() <= self.threshold.0 as usize,
                    "uncompressed packet length of {} exceeds compression threshold of {}",
                    r.len(),
                    self.threshold.0
                );

                let remaining_len = r.len();

                self.buf.advance(packet_len_len + 1);

                data = self.buf.split_to(remaining_len);
            }
        } else {
            self.buf.advance(packet_len_len);
            data = self.buf.split_to(packet_len as usize);
        }

        // Decode the leading packet ID.
        r = data;
        let packet_id = VarInt::decode(&mut r)
            .context("failed to decode packet ID")?
            .0;

        data.advance(data.len() - r.len());

        let def_static: Box<_> = data.iter().copied().collect();
        let def_static = Box::leak(def_static);

        Ok(Some(BorrowedPacketFrame {
            id: packet_id,
            body: def_static,
        }))
    }

    pub fn shift_excess(&mut self) {
        let read_position = self.buf.cursor;

        if read_position == 0 {
            return;
        }

        let excess_len = self.buf.inner.len() - read_position;

        self.buf.inner.copy_within(read_position.., 0);
        self.buf.inner.resize_with(excess_len, || unsafe {
            core::hint::unreachable_unchecked()
        });

        self.buf.cursor = 0;
    }

    /// Get the compression threshold.
    #[must_use]
    pub fn compression(&self) -> CompressionThreshold {
        self.threshold
    }

    /// Sets the compression threshold.
    pub fn set_compression(&mut self, threshold: CompressionThreshold) {
        self.threshold = threshold;
    }

    /// Queues a slice of bytes into the buffer.
    pub fn queue_slice(&mut self, bytes: &[u8]) {
        self.buf.inner.extend_from_slice(bytes);
    }
}
