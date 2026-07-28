//! Arbitrary bytes into the decoder, which is the one surface a stranger owns.
//!
//! Everything else in this crate is checked against bytes the real server
//! produced. That answers "do we agree with Mojang", and it says nothing about
//! what happens when the peer is not Mojang. A client can send anything at all
//! before it has authenticated, so the whole contract of [`FrameDecoder`] on
//! hostile input is: return an error, or return a packet, and never panic,
//! hang, or read out of bounds.
//!
//! This runs a fixed corpus from a seeded generator rather than a random one.
//! A gate that fuzzes differently on every run is a gate that fails for
//! somebody else on a case you cannot reproduce; `nix run .#fuzz` is the
//! unbounded search, and this is the part of it that has to stay green.
//!
//! The generator is deliberately weighted towards *nearly* valid frames.
//! Uniform random bytes almost always fail on the first length prefix and
//! never reach the compression layer, which is where the interesting arithmetic
//! is.

// The corpus generator is deliberately made of wrapping and truncating
// arithmetic: an LCG is defined by its overflow, and a `VarInt` writer is
// defined by chopping a `u32` into seven-bit pieces. Every cast below is the
// operation itself rather than a conversion that happens to be lossy, and a
// checked version would be a different function.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the generator's arithmetic is modular by definition"
)]

use hyperion_minecraft_proto::framing::{FrameDecoder, SHARED_SECRET_LEN};

/// The length prefix, written here rather than borrowed from the crate.
///
/// `framing::write_var_int` is private, and a fuzzer that built its inputs with
/// the encoder it is testing would agree with the decoder about any bug they
/// share. This is transcribed from `VarInt.write` instead.
fn write_var_int(out: &mut Vec<u8>, value: i32) {
    let mut remaining = value as u32;
    loop {
        if remaining & !0x7F == 0 {
            out.push(remaining as u8);
            return;
        }
        out.push((remaining as u8 & 0x7F) | 0x80);
        remaining >>= 7;
    }
}

/// Numerical Recipes' LCG. Fixed constants and fixed seeds, so the corpus is
/// the same on every machine and a failure is reproducible from the seed alone.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    const fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    const fn byte(&mut self) -> u8 {
        (self.next() & 0xFF) as u8
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }
}

/// How the decoder was configured when a case ran, so a failure can be
/// reproduced.
#[derive(Debug, Clone, Copy)]
struct Setup {
    threshold: Option<usize>,
    encrypted: bool,
    /// Bytes per `queue` call. One byte at a time exercises every partial
    /// frame state the decoder has.
    chunk: usize,
}

const fn setup(rng: &mut Rng) -> Setup {
    Setup {
        threshold: match rng.below(3) {
            0 => None,
            1 => Some(256),
            _ => Some(rng.below(4096) as usize),
        },
        encrypted: rng.below(4) == 0,
        chunk: 1 + rng.below(64) as usize,
    }
}

/// One input, built to look plausible often enough to get past the framing.
fn case(rng: &mut Rng) -> Vec<u8> {
    match rng.below(10) {
        // Uniform noise. Cheap, and it is what a port scanner sends.
        0 | 1 => {
            let len = rng.below(512) as usize;
            rng.bytes(len)
        }

        // A well formed length prefix over a random body, which is the shape
        // that actually reaches the id and body decode.
        2..=4 => {
            let len = rng.below(600) as usize;
            let body = rng.bytes(len);
            let mut out = Vec::new();
            write_var_int(&mut out, i32::try_from(body.len()).unwrap_or(0));
            out.extend_from_slice(&body);
            out
        }

        // A length prefix that lies: it declares far more or far less than
        // follows. Truncation and over-declaration are different bugs.
        5 | 6 => {
            let declared = rng.below(1 << 22) as i32;
            let mut out = Vec::new();
            write_var_int(&mut out, declared);
            let len = rng.below(300) as usize;
            out.extend_from_slice(&rng.bytes(len));
            out
        }

        // A compressed frame whose declared uncompressed length is chosen
        // adversarially. This is the zip-bomb shape: a handful of bytes on the
        // wire asking the decoder to materialise megabytes.
        7 | 8 => {
            let declared = match rng.below(4) {
                0 => 0,
                1 => rng.below(64) as i32,
                2 => 0x0080_0000,
                _ => rng.below(0x0100_0000) as i32,
            };
            let mut inner = Vec::new();
            write_var_int(&mut inner, declared);
            let len = rng.below(200) as usize;
            inner.extend_from_slice(&rng.bytes(len));

            let mut out = Vec::new();
            write_var_int(&mut out, i32::try_from(inner.len()).unwrap_or(0));
            out.extend_from_slice(&inner);
            out
        }

        // Several frames back to back, so a decoder that recovers its cursor
        // wrongly after one bad frame is caught on the next.
        _ => {
            let mut out = Vec::new();
            for _ in 0..rng.below(6) {
                let len = rng.below(64) as usize;
                let body = rng.bytes(len);
                write_var_int(&mut out, i32::try_from(body.len()).unwrap_or(0));
                out.extend_from_slice(&body);
            }
            out
        }
    }
}

/// Feed one case to a decoder configured by `setup`, draining until it stops.
///
/// Returns how many packets came out, which is not asserted on: the point is
/// that this returns at all.
fn drive(input: &[u8], setup: Setup) -> usize {
    let mut decoder = FrameDecoder::new();
    decoder.set_compression_threshold(setup.threshold);
    if setup.encrypted {
        decoder.enable_encryption(&[0x42; SHARED_SECRET_LEN]);
    }

    let mut packets = 0;
    for chunk in input.chunks(setup.chunk) {
        decoder.queue(chunk);
        // A bad frame leaves the stream position unknown, so a real connection
        // closes. Draining past the error here is deliberate: it is the
        // cheapest way to find a decoder that leaves its cursor somewhere it
        // cannot recover from.
        loop {
            match decoder.next_packet() {
                Ok(Some(packet)) => {
                    // Touch the body so a decoder handing back a slice it does
                    // not own is caught here rather than silently.
                    std::hint::black_box(packet.body.iter().fold(0u8, |a, b| a ^ b));
                    std::hint::black_box(packet.id);
                    packets += 1;
                }
                Ok(None) => break,
                Err(error) => {
                    std::hint::black_box(error.to_string());
                    break;
                }
            }
        }
    }
    packets
}

/// How much corpus to run, and from where.
///
/// The defaults are the gate: four thousand cases in about a second, the same
/// four thousand on every machine and every run. `nix run .#fuzz` raises them
/// and walks the seed base forward, which is the same generator searching
/// rather than the same generator checking.
///
/// A failure prints the seed, and the generator is a pure function of it, so a
/// case found by an hour-long run is reproduced by a one-second one.
fn corpus() -> (u64, u64, u64) {
    let read = |name: &str, fallback: u64| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    };
    (
        read("HYPERION_FUZZ_SEED_BASE", 0),
        read("HYPERION_FUZZ_SEEDS", 64),
        read("HYPERION_FUZZ_CASES", 64),
    )
}

#[test]
fn arbitrary_bytes_never_panic() {
    let (base, seeds, per_seed) = corpus();
    let mut cases = 0u64;
    for seed in base..base + seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
        for index in 0..per_seed {
            let setup = setup(&mut rng);
            let input = case(&mut rng);
            // A panic here escapes with the seed already printed above it, and
            // the seed is all that is needed to get this case back.
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive(&input, setup)));
            assert!(
                outcome.is_ok(),
                "seed {seed}, case {index}: the decoder panicked on {} bytes with {setup:?}",
                input.len()
            );
            cases += 1;
        }
    }
    assert_eq!(cases, seeds * per_seed, "the corpus did not run");
}

/// Every prefix of a valid stream is either incomplete or an error, never a
/// packet that is not there.
///
/// The partial-read path is where a decoder is most likely to read past what it
/// has, because it is the path a test written against whole frames never
/// exercises.
#[test]
fn a_truncated_stream_never_yields_a_packet_that_has_not_arrived() {
    use hyperion_minecraft_proto::framing::FrameEncoder;

    let mut encoder = FrameEncoder::new();
    let mut whole = Vec::new();
    for id in 0..8 {
        encoder
            .encode(id, &vec![id as u8; 40 + id as usize * 13], &mut whole)
            .expect("encode");
    }

    for cut in 0..whole.len() {
        let mut decoder = FrameDecoder::new();
        decoder.queue(&whole[..cut]);

        let mut seen = 0;
        while let Ok(Some(packet)) = decoder.next_packet() {
            assert_eq!(
                packet.id, seen,
                "packet ids came out of order from a {cut}-byte prefix"
            );
            seen += 1;
        }

        // Whatever came out must be a prefix of what a whole stream gives.
        let mut reference = FrameDecoder::new();
        reference.queue(&whole);
        let mut total = 0;
        while let Ok(Some(_)) = reference.next_packet() {
            total += 1;
        }
        assert!(
            seen <= total,
            "a {cut}-byte prefix produced {seen} packets, more than the whole {total}"
        );
    }
}

/// A frame declaring a length wider than 21 bits is refused rather than
/// buffered.
///
/// The check `Varint21FrameDecoder.copyVarint` makes, and the reason a peer
/// cannot ask the decoder to wait for four gigabytes.
#[test]
fn an_over_wide_length_prefix_is_refused_immediately() {
    let mut decoder = FrameDecoder::new();
    decoder.queue(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
    assert!(
        decoder.next_packet().is_err(),
        "a four-byte length prefix should be rejected"
    );
}
