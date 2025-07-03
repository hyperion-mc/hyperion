use std::{
    io::{Cursor, Write},
    mem::size_of,
};

use packet_channel::SendError;
use proptest::prelude::*;
use valence_protocol::{Encode, MAX_PACKET_SIZE, VarInt};

fn packets_and_splits() -> impl Strategy<Value = (Vec<Vec<u8>>, usize, Vec<usize>)> {
    let strategy = prop::collection::vec(prop::collection::vec(any::<u8>(), 1..10), 1..10)
        .prop_flat_map(|packets| {
            let total_len = packets
                .iter()
                .map(|packet| {
                    VarInt(i32::try_from(packet.len()).unwrap()).written_size() + packet.len()
                })
                .sum();
            (
                Just(packets),
                Just(total_len),
                prop::collection::vec(0..total_len, 0..10).prop_map(|mut splits| {
                    splits.sort_unstable();
                    splits
                }),
            )
        });

    // proptest uses an excessive amount of time to shrink these, so skip shrinking
    strategy.no_shrink()
}

fn encode(packets: &[Vec<u8>], total_len: usize) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::with_capacity(total_len));
    for packet in packets {
        VarInt(i32::try_from(packet.len()).unwrap())
            .encode(&mut cursor)
            .unwrap();
        cursor.write_all(packet).unwrap();
    }
    let encoded_packets = cursor.into_inner();
    assert_eq!(encoded_packets.len(), total_len);
    encoded_packets
}

fn for_each_segment(encoded_packets: &[u8], splits: &[usize], mut handler: impl FnMut(&[u8])) {
    if splits.is_empty() {
        handler(encoded_packets);
    } else {
        handler(&encoded_packets[0..*splits.first().unwrap()]);
        for range in splits.windows(2) {
            let &[start, end] = range else {
                panic!("windows should return a 2 length slice")
            };
            handler(&encoded_packets[start..end]);
        }
        handler(&encoded_packets[*splits.last().unwrap()..]);
    }
}

/// Helper function to allow implicitly coercing inputs to &[u8]
fn check(packet: &[u8], expected_packet: &[u8]) {
    assert_eq!(
        packet, expected_packet,
        "packet does not match expected packet"
    );
}

proptest! {
    #[test]
    fn test_packet_channel_valid_packets_unfragmented(
        (packets, total_len, splits) in packets_and_splits(),
    ) {
        let encoded_packets = encode(&packets, total_len);

        // This is different from total_len because the channel uses a fixed-size u32 to store
        // length while the input uses a VarInt
        let total_encoded_packet_size = packets.iter().map(|packet| size_of::<u32>() + packet.len()).sum();
        let (mut sender, mut receiver) = packet_channel::channel(total_encoded_packet_size);
        assert!(receiver.try_recv().is_none());
        std::thread::scope(|s| {
            s.spawn(
                || {
                    for_each_segment(&encoded_packets, &splits, |slice| sender.send(slice).unwrap());
                }
            );

            s.spawn(|| {
                let mut remaining_packets: &[Vec<u8>] = &packets;
                while let Some(expected_packet) = remaining_packets.first() {
                    if let Some(packet) = receiver.try_recv() {
                        assert_eq!(packet.fragment_id(), 0);
                        check(&packet, expected_packet);
                        remaining_packets = &remaining_packets[1..];
                    }
                }
            });
        });

        assert!(receiver.try_recv().is_none());
    }
}

proptest! {
    #[test]
    fn test_packet_channel_valid_packets_fragmented(
        (packets, total_len, splits) in packets_and_splits(),
         default_fragment_size in 0..10usize
    ) {
        let encoded_packets = encode(&packets, total_len);

        let (mut sender, mut receiver) = packet_channel::channel(default_fragment_size);
        assert!(receiver.try_recv().is_none());
        std::thread::scope(|s| {
            s.spawn(
                || {
                    for_each_segment(&encoded_packets, &splits, |slice| sender.send(slice).unwrap());
                }
            );

            s.spawn(|| {
                let mut remaining_packets: &[Vec<u8>] = &packets;
                let mut expected_fragment_id = 0;
                let mut bytes_remaining_in_current_fragment = default_fragment_size;
                while let Some(expected_packet) = remaining_packets.first() {
                    if let Some(packet) = receiver.try_recv() {
                        check(&packet, expected_packet);

                        // This checks internal implementation details about how new fragments are generated.
                        // This is subject to change.
                        let encoded_packet_size = size_of::<u32>() + expected_packet.len();
                        if bytes_remaining_in_current_fragment < encoded_packet_size {
                            // A new fragment should be allocated
                            expected_fragment_id += 1;
                            bytes_remaining_in_current_fragment = std::cmp::max(encoded_packet_size, default_fragment_size);
                        }
                        bytes_remaining_in_current_fragment -= encoded_packet_size;
                        assert_eq!(packet.fragment_id(), expected_fragment_id);

                        remaining_packets = &remaining_packets[1..];
                    }
                }
            });
        });

        assert!(receiver.try_recv().is_none());
    }
}

proptest! {
    #[test]
    fn test_packet_channel_zero_size_packets(
        (packets, total_len, splits) in packets_and_splits(),
        default_fragment_size in 0..10usize
    ) {
        let encoded_packets = encode(&packets, total_len);

        let (mut sender, mut receiver) = packet_channel::channel(default_fragment_size);
        assert!(receiver.try_recv().is_none());
        std::thread::scope(|s| {
            s.spawn(
                || {
                    for_each_segment(&encoded_packets, &splits, |slice| sender.send(slice).unwrap());
                    assert!(sender.send(&[]).is_ok());
                    assert_eq!(sender.send(&[0]), Err(SendError::ZeroLengthPacket));
                    assert_eq!(sender.send(&[]), Err(SendError::AlreadyClosed));
                    for_each_segment(&encoded_packets, &splits, |slice| assert_eq!(sender.send(slice), Err(SendError::AlreadyClosed)));
                }
            );

            s.spawn(|| {
                let mut remaining_packets: &[Vec<u8>] = &packets;
                while let Some(expected_packet) = remaining_packets.first() {
                    if let Some(packet) = receiver.try_recv() {
                        check(&packet, expected_packet);
                        remaining_packets = &remaining_packets[1..];
                    }
                }
            });
        });

        assert!(receiver.try_recv().is_none());
    }
}

proptest! {
    #[test]
    fn test_packet_channel_too_large_packets(
        (packets, total_len, splits) in packets_and_splits(),
        default_fragment_size in 0..10usize,
    ) {
        let encoded_packets = encode(&packets, total_len);

        // Write the max packet size as a VarInt
        let mut cursor = Cursor::new(Vec::new());
        VarInt(MAX_PACKET_SIZE).encode(&mut cursor).unwrap();
        let max_packet_size_encoded = cursor.into_inner();

        let (mut sender, mut receiver) = packet_channel::channel(default_fragment_size);
        assert!(receiver.try_recv().is_none());
        std::thread::scope(|s| {
            s.spawn(
                || {
                    for_each_segment(&encoded_packets, &splits, |slice| sender.send(slice).unwrap());
                    assert_eq!(sender.send(&max_packet_size_encoded), Err(SendError::TooLargePacket));
                    assert_eq!(sender.send(&[]), Err(SendError::AlreadyClosed));
                    for_each_segment(&encoded_packets, &splits, |slice| assert_eq!(sender.send(slice), Err(SendError::AlreadyClosed)));
                }
            );

            s.spawn(|| {
                let mut remaining_packets: &[Vec<u8>] = &packets;
                while let Some(expected_packet) = remaining_packets.first() {
                    if let Some(packet) = receiver.try_recv() {
                        check(&packet, expected_packet);
                        remaining_packets = &remaining_packets[1..];
                    }
                }
            });
        });

        assert!(receiver.try_recv().is_none());
    }
}
