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

use crate::net::packet_channel::RawPacket;

/// A buffer for saving bytes that are not yet decoded.
#[derive(Default, Component)]
pub struct PacketDecoder {
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
    pub fn try_next_packet<'b>(
        &'b mut self,
        bump: &'b bumpalo::Bump,
        raw_packet: &'b RawPacket,
    ) -> anyhow::Result<Option<BorrowedPacketFrame<'b>>> {
        let mut raw_packet: &[u8] = &raw_packet;
        let mut data;

        #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
        if self.threshold.0 >= 0 {
            let data_len = VarInt::decode(&mut raw_packet)?.0;

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

                    decompressor.zlib_decompress(raw_packet, decompression_buf)?
                };

                debug_assert_eq!(
                    written_len, data_len as usize,
                    "{written_len} != {data_len}"
                );

                data = &*decompression_buf;
            } else {
                debug_assert_eq!(data_len, 0, "{data_len} != 0");

                ensure!(
                    raw_packet.len() <= self.threshold.0 as usize,
                    "uncompressed packet length of {} exceeds compression threshold of {}",
                    raw_packet.len(),
                    self.threshold.0
                );

                data = raw_packet;
            }
        } else {
            data = raw_packet;
        }

        // Decode the leading packet ID.
        let packet_id = VarInt::decode(&mut data)
            .context("failed to decode packet ID")?
            .0;

        let def_static: Box<_> = data.iter().copied().collect();
        let def_static = Box::leak(def_static);

        Ok(Some(BorrowedPacketFrame {
            id: packet_id,
            body: def_static,
        }))
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
}
