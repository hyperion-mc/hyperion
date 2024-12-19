use std::io::{ErrorKind, Read, Write};

use crate::{Bot, Compression, Error, packet_processors, packet_utils::Buf};

pub fn read_socket(bot: &mut Bot, packet: &mut Buf) -> bool {
    if bot.kicked {
        return false;
    }

    let w_i = packet.get_writer_index();
    let result = bot.stream.read(&mut packet.buffer[w_i as usize..]);
    match result {
        Ok(0) => {
            tracing::warn!("Peer closed socket");
            bot.kicked = true;
            false
        }
        Ok(written) => {
            let written = u32::try_from(written).expect("written is not a u32");
            packet.set_writer_index(packet.get_writer_index() + written);
            true
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            // Break out of loop
            false
        }
        Err(e) => {
            tracing::warn!("unable to read socket: {e:?}");
            bot.kicked = true;
            false
        }
    }
}

pub fn buffer(temp_buf: &Buf, buffering_buf: &mut Buf) {
    buffering_buf.write_bytes(
        &temp_buf.buffer
            [temp_buf.get_reader_index() as usize..temp_buf.get_writer_index() as usize],
    );
}

pub fn unbuffer(temp_buf: &mut Buf, buffering_buf: &mut Buf) {
    if buffering_buf.get_writer_index() != 0 {
        temp_buf.write_bytes(&buffering_buf.buffer[..buffering_buf.get_writer_index() as usize]);
        buffering_buf.set_writer_index(0);
    }
}

pub fn process_packet(
    bot: &mut Bot,
    packet_buf: &mut Buf,
    decompression_buf: &mut Buf,
    compression: &mut Compression,
) {
    packet_buf.set_reader_index(0);
    packet_buf.set_writer_index(0);

    // Read new packets
    unbuffer(packet_buf, &mut bot.buffering_buf);
    while read_socket(bot, packet_buf) {
        let len = packet_buf.buffer.len();

        // Reallocate if full
        #[allow(clippy::uninit_vec)]
        if packet_buf.get_writer_index() == u32::try_from(len).expect("len is not a u32") {
            packet_buf.buffer.reserve(len);
            unsafe {
                packet_buf.buffer.set_len(len * 2);
            }
        }
    }
    if bot.kicked {
        return;
    }

    let mut next = 0;

    // Process all of the Minecraft packets received
    loop {
        // Handle packet that have an incomplete size field
        if packet_buf.get_writer_index() - next < 3 {
            buffer(packet_buf, &mut bot.buffering_buf);
            break;
        }

        // Read packet size
        let tuple = packet_buf.read_var_u32();
        let size = tuple.0 as usize;
        next += tuple.0 + tuple.1;

        // Skip packets of 0 length
        if size == 0 {
            tracing::warn!("0 len packet (shouldn't be possible)");
            continue;
        }

        // Handle incomplete packet
        if packet_buf.get_writer_index() < size as u32 + packet_buf.get_reader_index() {
            packet_buf.set_reader_index(packet_buf.get_reader_index() - tuple.1);
            buffer(packet_buf, &mut bot.buffering_buf);
            break;
        }

        // Decompress if needed and parse the packet
        if bot.compression_threshold > 0 {
            let real_length_tuple = packet_buf.read_var_u32();
            let real_length = real_length_tuple.0;

            // Buffer is compressed
            if real_length != 0 {
                decompression_buf.set_reader_index(0);
                decompression_buf.set_writer_index(0);

                {
                    let start = packet_buf.get_reader_index() as usize;
                    let end = packet_buf.get_reader_index() as usize + size
                        - real_length_tuple.1 as usize;

                    if start > end {
                        tracing::warn!(
                            "s {} > e {}, size: {}, tl: {}, ri: {}, wi: {}",
                            start,
                            end,
                            size,
                            real_length_tuple.1,
                            packet_buf.get_reader_index(),
                            packet_buf.get_writer_index()
                        );
                        bot.kicked = true;
                        break;
                    }

                    // Decompress
                    match decompress_packet(
                        real_length,
                        &packet_buf.buffer[start..end],
                        compression,
                        decompression_buf,
                    ) {
                        Ok(x) => x,
                        Err(err) => {
                            tracing::warn!("decompression error: {err}");
                            bot.kicked = true;
                            break;
                        }
                    };
                }

                packet_processors::process_decode(decompression_buf, bot, compression);
            } else {
                packet_processors::process_decode(packet_buf, bot, compression);
            }
        } else {
            packet_processors::process_decode(packet_buf, bot, compression);
        }
        if bot.kicked {
            break;
        }

        // Prepare for next packet and exit condition
        packet_buf.set_reader_index(next);
        if packet_buf.get_reader_index() >= packet_buf.get_writer_index() {
            break;
        }
    }
}

impl Bot {
    pub fn send_packet(&mut self, buf: Buf, compression: &mut Compression) {
        if self.kicked {
            return;
        }
        let mut packet = buf;
        if self.compression_threshold > 0 {
            packet = packet_processors::PacketCompressor::process_write(&packet, self, compression)
                .unwrap();
        }
        packet = packet_processors::PacketFramer::process_write(&packet);
        match self.stream.write_all(
            &packet.buffer[packet.get_reader_index() as usize..packet.get_writer_index() as usize],
        ) {
            Ok(()) => {}
            Err(e) => {
                self.kicked = true;
                tracing::warn!("could not write to buf: {e}");
            }
        }
    }
}

pub fn decompress_packet(
    real_length: u32,
    working_buf: &[u8],
    compression: &mut Compression,
    compression_buffer: &mut Buf,
) -> Result<(), Error> {
    compression_buffer.ensure_writable(real_length);
    let range = compression_buffer.get_writer_index() as usize..;

    // decompress
    let written = compression
        .decompressor
        .zlib_decompress(working_buf, &mut compression_buffer.buffer[range])?;
    assert_eq!(
        written, real_length as usize,
        "written != real_length, written: {written}, real_length: {real_length}"
    );
    compression_buffer.set_writer_index(real_length);

    Ok(())
}
