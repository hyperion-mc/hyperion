use std::{hint::black_box, sync::mpsc};

use divan::Bencher;

const LENS: &[usize] = &[1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

fn main() {
    divan::main();
}

// This isn't a completely fair comparison since mpsc::sync_channel is a bounded channel and
// packet_channel is an unbounded channel and this mpsc variant is skipping the VarInt length prefix
// decoding that packet_channel does, but packet_channel is still faster despite this

#[divan::bench(
    args = LENS,
)]
fn send_packet_mpsc(bencher: Bencher<'_, '_>, len: usize) {
    let packet = [0u8; 64];
    let (writer, reader) = mpsc::sync_channel(*LENS.last().unwrap());

    bencher.counter(len).bench_local(|| {
        for _ in 0..black_box(len) {
            writer
                .send(Box::<[u8]>::from(black_box(packet).as_slice()))
                .unwrap();
        }
        let mut total_packets = 0;
        while let Ok(packet) = reader.try_recv() {
            black_box(packet);
            total_packets += 1;
        }
        assert_eq!(total_packets, len);
    });
}

#[divan::bench(
    args = LENS,
)]
fn send_packet_channel(bencher: Bencher<'_, '_>, len: usize) {
    let mut packet = [0u8; 65];
    // Add the length prefix
    packet[0] = 64;

    let (mut writer, mut reader) = packet_channel::channel(4096);

    bencher.counter(len).bench_local(|| {
        for _ in 0..black_box(len) {
            writer.send(&black_box(packet)[..]).unwrap();
        }
        let mut total_packets = 0;
        while let Some(packet) = reader.try_recv() {
            black_box(packet);
            total_packets += 1;
        }
        assert_eq!(total_packets, len);
    });
}
