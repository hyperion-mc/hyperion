use anyhow::{Context, ensure};
use bevy::prelude::*;
use bytes::{Bytes, BytesMut};
use itertools::Either;
use valence_protocol::{
    CompressionThreshold, Decode, DecodeBytes, MAX_PACKET_SIZE, Packet, VarInt,
};

use crate::net::packet_channel::RawPacket;

/// A buffer for saving bytes that are not yet decoded.
#[derive(Default, Component)]
pub struct PacketDecoder {
    threshold: CompressionThreshold,
}

#[derive(Clone)]
pub struct BorrowedPacketFrame {
    /// The ID of the decoded packet.
    pub id: i32,
    /// The contents of the packet after the leading [`VarInt`] ID. This stores either a [`Bytes`]
    /// or [`RawPacket`] because [`Bytes::from_owner`] has some performance penalty and requires an
    /// allocation to store metadata.
    pub body: Either<Bytes, RawPacket>,
}

impl BorrowedPacketFrame {
    /// Attempts to decode this packet as type `P`. An error is returned if the
    /// packet ID does not match, the body of the packet failed to decode, or
    /// some input was missed.
    pub fn decode<P>(self) -> anyhow::Result<P>
    where
        P: Packet + DecodeBytes,
    {
        ensure!(
            P::ID == self.id,
            "packet ID mismatch while decoding '{}': expected {}, got {}",
            P::NAME,
            P::ID,
            self.id
        );

        let pkt = match self.body {
            Either::Left(mut bytes) => {
                let pkt = P::decode_bytes(&mut bytes)?;

                ensure!(
                    bytes.is_empty(),
                    "missed {} bytes while decoding '{}'",
                    bytes.len(),
                    P::NAME
                );

                pkt
            }
            Either::Right(packet) => {
                let initial_len = packet.len();
                let (pkt, bytes_read) = P::decode_from_owned(packet)?;

                ensure!(
                    bytes_read == initial_len,
                    "missed {} bytes while decoding '{}'",
                    initial_len - bytes_read,
                    P::NAME
                );

                pkt
            }
        };

        Ok(pkt)
    }
}

impl PacketDecoder {
    pub fn try_next_packet(
        &self,
        decompressor: &mut libdeflater::Decompressor,
        mut raw_packet: RawPacket,
    ) -> anyhow::Result<BorrowedPacketFrame> {
        let mut raw_packet_slice: &[u8] = &raw_packet;
        let mut data;

        #[expect(clippy::cast_sign_loss, reason = "we are checking if < 0")]
        if self.threshold.0 >= 0 {
            let data_len = VarInt::decode(&mut raw_packet_slice)?.0;

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

                // todo(perf): find a decompression library which accepts &[MaybeUninit<u8>] to
                // avoid cost of initializing the data
                let mut decompression_buf = BytesMut::zeroed(usize::try_from(data_len)?);

                let written_len =
                    decompressor.zlib_decompress(raw_packet_slice, &mut decompression_buf)?;

                debug_assert_eq!(
                    written_len, data_len as usize,
                    "{written_len} != {data_len}"
                );

                data = Either::Left(decompression_buf.freeze());
            } else {
                debug_assert_eq!(data_len, 0, "{data_len} != 0");

                ensure!(
                    raw_packet_slice.len() <= self.threshold.0 as usize,
                    "uncompressed packet length of {} exceeds compression threshold of {}",
                    raw_packet_slice.len(),
                    self.threshold.0
                );

                // Remove the initial VarInt from raw_packet
                let bytes_read = raw_packet.len() - raw_packet_slice.len();
                raw_packet.remove_front(bytes_read);
                data = Either::Right(raw_packet);
            }
        } else {
            data = Either::Right(raw_packet);
        }

        // Decode the leading packet ID.
        let packet_id = match &mut data {
            Either::Left(bytes) => {
                VarInt::decode_bytes(bytes)
                    .context("failed to decode packet ID")?
                    .0
            }
            Either::Right(packet) => {
                let (VarInt(id), bytes_read) =
                    VarInt::decode_and_len(packet).context("failed to decode packet ID")?;
                packet.remove_front(bytes_read);
                id
            }
        };

        Ok(BorrowedPacketFrame {
            id: packet_id,
            body: data,
        })
    }

    /// Get the compression threshold.
    #[must_use]
    pub const fn compression(&self) -> CompressionThreshold {
        self.threshold
    }

    /// Sets the compression threshold.
    pub const fn set_compression(&mut self, threshold: CompressionThreshold) {
        self.threshold = threshold;
    }
}
