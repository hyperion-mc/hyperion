//! How much memory can a stranger make the decoder allocate per byte they send?
//!
//! "Never panics" is half of what a decoder on an unauthenticated socket owes
//! you. The other half is that the memory it takes is a function of what
//! arrived and not of what the sender claimed, because a peer who can ask for
//! an allocation with a few bytes has a denial of service whether or not any
//! single allocation is bounded.
//!
//! This measures it rather than reasoning about it, with a global allocator
//! that counts. The counter is process wide, so this binary holds exactly one
//! test: `cargo test` runs a binary's tests as threads in one process, and a
//! second test allocating alongside this one is measured as part of it. That is
//! not hypothetical: an earlier version of this file had two tests, and the
//! trickle measurement read 140 KB under `cargo test` against 76 KB when it has
//! the process to itself. It passed under nextest, which gives every test its
//! own process, and failed in the coverage job, which does not.
//!
//! # What it found
//!
//! The compression layer is a genuine amplifier, and this pins the size of it.
//! A compressed frame carries a `VarInt` saying how many bytes it inflates to,
//! and `Decompressor::inflate` allocates and zeroes exactly that many before
//! looking at the payload at all. Measured, on the sweep this test runs:
//!
//! ```text
//! declared       1024:  7 bytes on the wire ->     44,336 bytes allocated
//! declared  1,048,576:  8 bytes on the wire ->  1,091,888 bytes allocated
//! declared  8,388,608:  9 bytes on the wire ->  8,431,921 bytes allocated
//! declared  8,388,609:  9 bytes on the wire ->     43,313 bytes allocated
//! ```
//!
//! So nine bytes from an unauthenticated peer buy an 8.4 MB allocation, a ratio
//! of about 900,000 to one. The last row is the one that makes it survivable:
//! one byte over `MAXIMUM_UNCOMPRESSED_LENGTH` and the frame is refused on its
//! declared length alone, before anything is reserved. Roughly 43 KB is the
//! decoder's own fixed setup and is the floor every row sits on.
//!
//! None of that is a bug in this crate. It is what `CompressionDecoder.inflate`
//! does, and the 8 MiB ceiling is Mojang's own `MAXIMUM_UNCOMPRESSED_LENGTH`;
//! faithfulness to the server is the point of this crate, so the number is
//! pinned rather than lowered. What this test defends is that the bound is a
//! constant rather than the declared length, which is exactly what stops being
//! true if the check on the last row is ever removed.
//!
//! It is still worth knowing at the connection level: the ceiling is per
//! packet, not per connection, so a peer that keeps sending them keeps buying
//! 8 MB at a time. Rate limiting is the proxy's business rather than the
//! decoder's, but nothing here makes it unnecessary.

// The `VarInt` writer chops a `u32` into seven-bit pieces, so the truncation is
// the encoding rather than an accident of it.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a VarInt is defined by taking the low seven bits at a time"
)]

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

use hyperion_minecraft_proto::framing::{FrameDecoder, MAX_UNCOMPRESSED_LENGTH};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every method forwards to `System` with the same layout it was given,
// so the allocator contract is whatever `System`'s is. The counters are plain
// atomics and touch no allocation state.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged.
        let out = unsafe { System.realloc(pointer, layout, new_size) };
        if !out.is_null() {
            let live = LIVE
                .fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed)
                .saturating_add(new_size.saturating_sub(layout.size()));
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        out
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Peak live bytes during `body`, over the level it was already at.
fn peak_of(body: impl FnOnce()) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    body();
    PEAK.load(Ordering::Relaxed).saturating_sub(before)
}

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

/// A compressed frame that claims to inflate to `declared` bytes and carries
/// almost nothing.
fn lying_frame(declared: i32) -> Vec<u8> {
    let mut inner = Vec::new();
    write_var_int(&mut inner, declared);
    inner.extend_from_slice(&[0x78, 0x9C, 0x03, 0x00]);

    let mut out = Vec::new();
    write_var_int(&mut out, i32::try_from(inner.len()).unwrap_or(0));
    out.extend_from_slice(&inner);
    out
}

fn drain(decoder: &mut FrameDecoder) {
    loop {
        match decoder.next_packet() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return,
        }
    }
}

#[test]
fn what_a_peer_can_make_the_decoder_allocate_is_bounded() {
    // Warm the allocator and the decompressor's own buffers first, so what is
    // measured is the decode and not one-off setup.
    {
        let mut decoder = FrameDecoder::new();
        decoder.set_compression_threshold(Some(256));
        decoder.queue(&lying_frame(1024));
        drain(&mut decoder);
    }

    ceiling_holds_however_much_is_declared();
    buffered_bytes_grow_with_what_arrived();
}

/// A declared length cannot buy more than the documented ceiling.
fn ceiling_holds_however_much_is_declared() {
    // The declared length is what the peer controls, so it is what is swept.
    // Every one of these frames is under ten bytes on the wire.
    let ceiling = MAX_UNCOMPRESSED_LENGTH;
    for declared in [
        1_024,
        64 * 1_024,
        1_024 * 1_024,
        i32::try_from(ceiling).expect("the ceiling fits in an i32"),
        // Past the ceiling. This must be refused before anything is allocated,
        // which is the check that makes the ceiling mean anything.
        i32::try_from(ceiling).expect("the ceiling fits in an i32") + 1,
        i32::MAX,
    ] {
        let frame = lying_frame(declared);
        let wire = frame.len();

        let peak = peak_of(|| {
            let mut decoder = FrameDecoder::new();
            decoder.set_compression_threshold(Some(256));
            decoder.queue(&frame);
            drain(&mut decoder);
        });

        // Printed rather than only asserted: the ratio is the finding, and a
        // reader running this with --no-capture should be able to see it
        // rather than take the module comment's word for it.
        eprintln!(
            "declared {declared:>10}: {wire} bytes on the wire -> {peak} bytes allocated ({}x)",
            peak / wire.max(1)
        );

        assert!(
            peak <= ceiling + 64 * 1_024,
            "{wire} bytes on the wire declaring {declared} caused a peak of {peak} bytes, above \
             the {ceiling}-byte ceiling"
        );

        // Anything over the ceiling is rejected on the declared length alone,
        // so it never reaches the allocation at all.
        if usize::try_from(declared).is_ok_and(|declared| declared > ceiling) {
            assert!(
                peak < 1_024 * 1_024,
                "a frame declaring {declared}, which is over the ceiling, still allocated {peak} \
                 bytes"
            );
        }
    }
}

/// Bytes that never form a whole frame are held, not multiplied.
///
/// A peer that opens a connection and dribbles bytes without ever completing a
/// frame is the cheapest attack there is, so the decoder's buffer has to grow
/// with what arrived rather than with what was promised.
fn buffered_bytes_grow_with_what_arrived() {
    // A frame header promising nearly two megabytes, just under the 21-bit
    // frame cap, followed by a trickle that never finishes it.
    const PROMISED: i32 = 1_900_000;
    let mut frame = Vec::new();
    write_var_int(&mut frame, PROMISED);
    let trickle = vec![0xABu8; 32 * 1_024];
    let arrived = frame.len() + trickle.len();

    let peak = peak_of(|| {
        let mut decoder = FrameDecoder::new();
        decoder.queue(&frame);
        for chunk in trickle.chunks(256) {
            decoder.queue(chunk);
            drain(&mut decoder);
        }
        assert!(
            decoder.buffered_len() <= arrived,
            "the decoder is holding more than it was sent"
        );
    });
    eprintln!("promised {PROMISED}, sent {arrived} -> {peak} bytes allocated");

    // The claim is that the promise buys nothing, so the bound is stated
    // against the promise. A decoder that reserved what it was told to would
    // be an order of magnitude over this.
    assert!(
        peak < usize::try_from(PROMISED).expect("positive") / 4,
        "a promise of {PROMISED} bytes with only {arrived} sent caused a peak of {peak} bytes"
    );
    // And against what arrived, with room for a `Vec` that doubles and for
    // whatever instrumentation the build carries.
    assert!(
        peak <= arrived * 8,
        "{arrived} bytes of trickle caused a peak of {peak} bytes"
    );
}
