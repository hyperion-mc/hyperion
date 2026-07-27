//! Framing, against bytes from the server's own netty handlers.
//!
//! Two kinds of check. The fixture tests compare this encoder's output with
//! what `Varint21LengthFieldPrepender`, `CompressionEncoder` and
//! `CipherEncoder` produced for the same input; the loopback tests push a
//! packet through all three layers and back, which is what catches a decoder
//! that agrees with a matching bug in the encoder.

mod vanilla_fixtures;

use hyperion_minecraft_proto::framing::{
    Error, FrameDecoder, FrameEncoder, MAX_FRAME_LENGTH, SHARED_SECRET_LEN,
};

use vanilla_fixtures as vanilla;

/// The threshold `server.properties` ships with.
const VANILLA_THRESHOLD: usize = 256;

fn secret() -> [u8; SHARED_SECRET_LEN] {
    let bytes = vanilla::bytes("secret");
    bytes.try_into().expect("secret is 16 bytes")
}

fn encode_one(encoder: &mut FrameEncoder, packet_id: i32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encoder.encode(packet_id, body, &mut out).expect("encode");
    out
}

// --- against vanilla bytes -------------------------------------------------

#[test]
fn plain_frames_match_the_prepender() {
    let mut encoder = FrameEncoder::new();

    assert_eq!(
        vanilla::hex(&encode_one(&mut encoder, 0x00, b"")),
        vanilla::get("frame.plain.empty_body")
    );
    assert_eq!(
        vanilla::hex(&encode_one(&mut encoder, 0x2A, &[0x11; 8])),
        vanilla::get("frame.plain.small")
    );
}

#[test]
fn a_body_below_the_threshold_is_sent_uncompressed() {
    let mut encoder = FrameEncoder::new();
    encoder.set_compression_threshold(Some(VANILLA_THRESHOLD));

    let framed = encode_one(&mut encoder, 0x2A, &[0x11; 8]);
    assert_eq!(
        vanilla::hex(&framed),
        vanilla::get("frame.compressed_256.below")
    );
    // The zero after the frame length is the marker, not a length.
    assert_eq!(framed[1], 0x00);
}

/// Deflate output is not compared byte for byte, and cannot be: `zlib` and
/// `miniz_oxide` both emit valid streams at level 6 and they are not the same
/// stream. What is compared instead is the part the protocol fixes -- the
/// frame length, the declared uncompressed length -- plus the two directions
/// of interoperability: vanilla's bytes read back correctly here, and ours
/// read back to the same packet.
#[test]
fn a_body_above_the_threshold_is_deflated() {
    let body = [0x22u8; 512];
    let mut encoder = FrameEncoder::new();
    encoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    let ours = encode_one(&mut encoder, 0x2A, &body);

    let theirs = vanilla::bytes("frame.compressed_256.above");
    // Byte 0 is the frame length and bytes 1..3 are the declared uncompressed
    // length, 513 as a two-byte VarInt. Only the frame length may differ.
    assert_eq!(&ours[1..3], &theirs[1..3], "declared uncompressed length");
    assert_ne!(ours[1], 0x00, "past the threshold, so deflated");

    let mut decoder = FrameDecoder::new();
    decoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    decoder.queue(&theirs);
    let packet = decoder
        .next_packet()
        .expect("decode vanilla")
        .expect("a whole frame");
    assert_eq!(packet.id, 0x2A);
    assert_eq!(packet.body, &body[..]);

    decoder.queue(&ours);
    let packet = decoder
        .next_packet()
        .expect("decode ours")
        .expect("a whole frame");
    assert_eq!(packet.id, 0x2A);
    assert_eq!(packet.body, &body[..]);
}

/// `CompressionEncoder.encode` compares with `<`, so a packet whose id and
/// body together are exactly the threshold is compressed. This is the case the
/// wiki has historically described the other way round, and it is one byte
/// wide, so nothing but a fixture settles it.
#[test]
fn the_threshold_boundary_is_inclusive() {
    let mut encoder = FrameEncoder::new();
    encoder.set_compression_threshold(Some(64));

    let exact = encode_one(&mut encoder, 0x2A, &[0x33; 63]);
    let theirs = vanilla::bytes("frame.compressed_64.exact");
    assert_ne!(exact[1], 0x00, "a body of exactly the threshold is deflated");
    assert_ne!(theirs[1], 0x00, "and vanilla agrees");
    assert_eq!(exact[1], theirs[1], "both declare 64 uncompressed bytes");

    let under = encode_one(&mut encoder, 0x2A, &[0x33; 62]);
    assert_eq!(
        vanilla::hex(&under),
        vanilla::get("frame.compressed_64.just_under")
    );
    assert_eq!(under[1], 0x00, "one byte short of the threshold is not");
}

#[test]
fn encryption_matches_the_cipher_encoder() {
    let mut encoder = FrameEncoder::new();
    encoder.enable_encryption(&secret());
    assert_eq!(
        vanilla::hex(&encode_one(&mut encoder, 0x2A, &[0x11; 8])),
        vanilla::get("frame.encrypted.plain_small")
    );

    // The compressed case cannot be compared byte for byte through the cipher
    // either, for the deflate reason above: a different compressed length
    // shifts every byte after it. Reading vanilla's is the check that counts.
    let mut decoder = FrameDecoder::new();
    decoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    decoder.enable_encryption(&secret());
    decoder.queue(&vanilla::bytes("frame.encrypted.compressed_256_above"));
    let packet = decoder
        .next_packet()
        .expect("decode")
        .expect("a whole frame");
    assert_eq!(packet.id, 0x2A);
    assert_eq!(packet.body, &[0x22u8; 512][..]);
}

/// CFB8 carries its shift register between frames, so the second frame of a
/// stream is not what enciphering it on its own would give. A cipher created
/// per packet would pass every single-frame test above and fail this one.
#[test]
fn the_cipher_carries_across_frames() {
    let mut encoder = FrameEncoder::new();
    encoder.enable_encryption(&secret());

    let first = encode_one(&mut encoder, 0x01, &[0x11; 8]);
    let second = encode_one(&mut encoder, 0x02, &[0x11; 8]);

    assert_eq!(
        vanilla::hex(&first),
        vanilla::get("frame.encrypted.stream_first")
    );
    assert_eq!(
        vanilla::hex(&second),
        vanilla::get("frame.encrypted.stream_second")
    );

    let mut fresh = FrameEncoder::new();
    fresh.enable_encryption(&secret());
    assert_ne!(
        vanilla::hex(&encode_one(&mut fresh, 0x02, &[0x11; 8])),
        vanilla::get("frame.encrypted.stream_second"),
        "a per-packet cipher would produce these bytes and be wrong"
    );
}

// --- loopback --------------------------------------------------------------

/// Every combination of the two optional layers, over a range of body sizes
/// that straddles the threshold.
#[test]
fn loopback_round_trips_every_layer_combination() {
    for compression in [None, Some(0), Some(64), Some(VANILLA_THRESHOLD)] {
        for encrypted in [false, true] {
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::new();
            encoder.set_compression_threshold(compression);
            decoder.set_compression_threshold(compression);
            if encrypted {
                encoder.enable_encryption(&secret());
                decoder.enable_encryption(&secret());
            }

            for size in [0usize, 1, 63, 64, 65, 255, 256, 257, 4096] {
                // 251 rather than 256 so the pattern is not a multiple of any
                // buffer size the codec might use, which is what would hide an
                // off-by-one in the packing.
                let body: Vec<u8> = (0..size)
                    .map(|index| u8::try_from(index % 251).expect("below 251"))
                    .collect();
                let mut wire = Vec::new();
                encoder.encode(0x2A, &body, &mut wire).expect("encode");
                decoder.queue(&wire);

                let packet = decoder
                    .next_packet()
                    .expect("decode")
                    .expect("a whole frame was queued");
                assert_eq!(packet.id, 0x2A, "{compression:?} {encrypted} {size}");
                assert_eq!(packet.body, &body[..], "{compression:?} {encrypted} {size}");
            }
            assert_eq!(decoder.buffered_len(), 0, "no bytes left over");
        }
    }
}

/// A socket read is not a frame. Feeding the stream one byte at a time is the
/// worst case for the length-prefix reader, which has to hold state across
/// calls without consuming anything it cannot yet use.
#[test]
fn loopback_survives_arbitrary_read_boundaries() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    encoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    decoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    encoder.enable_encryption(&secret());
    decoder.enable_encryption(&secret());

    let bodies: Vec<Vec<u8>> = (0..16u8)
        .map(|packet| vec![packet; usize::from(packet) * 97])
        .collect();

    let mut wire = Vec::new();
    for (packet, body) in bodies.iter().enumerate() {
        encoder
            .encode(i32::try_from(packet).unwrap(), body, &mut wire)
            .expect("encode");
    }

    let mut received = Vec::new();
    for byte in &wire {
        decoder.queue(std::slice::from_ref(byte));
        while let Some(packet) = decoder.next_packet().expect("decode") {
            received.push((packet.id, packet.body.to_vec()));
        }
    }

    assert_eq!(received.len(), bodies.len());
    for (packet, (id, body)) in received.iter().enumerate() {
        assert_eq!(*id, i32::try_from(packet).unwrap());
        assert_eq!(body, &bodies[packet]);
    }
}

#[test]
fn several_frames_arrive_in_one_read() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();

    let mut wire = Vec::new();
    for packet in 0..8u8 {
        encoder
            .encode(i32::from(packet), &[packet; 40], &mut wire)
            .expect("encode");
    }
    decoder.queue(&wire);

    for packet in 0..8u8 {
        let received = decoder.next_packet().expect("decode").expect("a frame");
        assert_eq!(received.id, i32::from(packet));
        assert_eq!(received.body, &[packet; 40][..]);
    }
    assert!(decoder.next_packet().expect("decode").is_none());
}

/// Compression is negotiated mid-connection: `login_compression` is itself
/// sent uncompressed, and every frame after it is not.
#[test]
fn the_threshold_can_be_turned_on_between_frames() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();

    let mut wire = Vec::new();
    encoder.encode(0x03, b"login_compression", &mut wire).expect("encode");
    encoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    encoder.encode(0x02, &[0x55; 1024], &mut wire).expect("encode");

    decoder.queue(&wire);
    let first = decoder.next_packet().expect("decode").expect("a frame");
    assert_eq!(first.id, 0x03);
    assert_eq!(first.body, b"login_compression");

    decoder.set_compression_threshold(Some(VANILLA_THRESHOLD));
    let second = decoder.next_packet().expect("decode").expect("a frame");
    assert_eq!(second.id, 0x02);
    assert_eq!(second.body, &[0x55; 1024][..]);
}

// --- refusals --------------------------------------------------------------

#[test]
fn a_zero_length_frame_is_refused() {
    let mut decoder = FrameDecoder::new();
    decoder.queue(&[0x00]);
    assert_eq!(decoder.next_packet(), Err(Error::EmptyFrame));
}

#[test]
fn a_four_byte_length_prefix_is_refused() {
    let mut decoder = FrameDecoder::new();
    decoder.queue(&[0xFF, 0xFF, 0xFF, 0x7F]);
    assert_eq!(decoder.next_packet(), Err(Error::FrameLengthTooWide));
}

/// A 21-bit prefix can describe a frame larger than the splitter's own limit,
/// so the limit is checked rather than inferred from the prefix width.
#[test]
fn a_frame_past_the_21_bit_limit_is_refused() {
    let mut decoder = FrameDecoder::new();
    decoder.queue(&[0xFF, 0xFF, 0xFF]);
    match decoder.next_packet() {
        Err(Error::FrameLengthTooWide | Error::FrameTooLarge { .. }) => {}
        other => panic!("expected a refusal, got {other:?}"),
    }
    const { assert!(MAX_FRAME_LENGTH < (1 << 21)) };
}

/// `CompressionDecoder` rejects a frame whose declared length is below the
/// threshold: the sender should have used the zero marker, so accepting it
/// would leave two encodings for the same bytes.
#[test]
fn a_frame_compressed_below_the_threshold_is_refused() {
    let mut encoder = FrameEncoder::new();
    encoder.set_compression_threshold(Some(1));
    let mut wire = Vec::new();
    encoder.encode(0x2A, &[0x11; 8], &mut wire).expect("encode");

    let mut decoder = FrameDecoder::new();
    decoder.set_compression_threshold(Some(1024));
    decoder.queue(&wire);
    assert_eq!(
        decoder.next_packet(),
        Err(Error::BadlyCompressed {
            declared: 9,
            threshold: 1024
        })
    );
}

/// A frame claiming to inflate to more than it does is the zip-bomb shape.
#[test]
fn a_frame_that_lies_about_its_uncompressed_length_is_refused() {
    let mut encoder = FrameEncoder::new();
    encoder.set_compression_threshold(Some(8));
    let mut wire = Vec::new();
    encoder.encode(0x2A, &[0x11; 64], &mut wire).expect("encode");

    // Byte 0 is the frame length and byte 1 the declared uncompressed length,
    // both single-byte VarInts here. Raising the declared length leaves the
    // deflate stream intact and only the claim wrong.
    wire[1] = 100;
    let mut decoder = FrameDecoder::new();
    decoder.set_compression_threshold(Some(8));
    decoder.queue(&wire);
    assert_eq!(
        decoder.next_packet(),
        Err(Error::LengthMismatch {
            declared: 100,
            actual: 65
        })
    );
}
