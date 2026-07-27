#![allow(
    clippy::print_stdout,
    reason = "the purpose of not having printing to stdout is so that tracing is used properly \
              for the core libraries. These are tests, so it doesn't matter"
)]

use std::borrow::Cow;

use bvh::{Aabb, Bvh, Data, Point};
use glam::I16Vec2;
use itertools::Itertools;
use proptest::prelude::*;

#[derive(Clone)]
struct ChunkWithPackets<'a> {
    location: I16Vec2,
    packets_data: Cow<'a, [u8]>,
}

impl std::fmt::Debug for ChunkWithPackets<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {:?}", self.location, self.packets_data)
    }
}

impl Default for ChunkWithPackets<'_> {
    fn default() -> Self {
        Self {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        }
    }
}

impl Point for ChunkWithPackets<'_> {
    // todo: test returning val vs ref
    fn point(&self) -> I16Vec2 {
        self.location
    }
}

impl Data for ChunkWithPackets<'_> {
    type Unit = u8;

    fn data<'a: 'c, 'b: 'c, 'c>(&'a self, _context: ()) -> &'c [u8] {
        &self.packets_data
    }
}

#[test]
fn test_local_packet() {
    let data = [1, 2, 3, 4];
    let packet = ChunkWithPackets {
        location: I16Vec2::new(1, 2),
        packets_data: Cow::Borrowed(&data),
    };

    assert_eq!(packet.point(), I16Vec2::new(1, 2));
    assert_eq!(packet.data(()), &data);
}

#[test]
fn test_build_bvh_with_empty_input() {
    let mut data: Vec<ChunkWithPackets<'_>> = vec![];

    let bvh = Bvh::build(&mut data, ());

    assert_eq!(bvh.elements().len(), 0);

    assert_eq!(bvh.get_closest_slice(I16Vec2::new(0, 0)), None);

    assert_eq!(
        bvh.get_in(Aabb::new(I16Vec2::new(0, 0), I16Vec2::new(100, 100)))
            .len(),
        0
    );
}

#[test]
fn test_build_bvh_1_packet() {
    let data = [1];
    let packet = ChunkWithPackets {
        location: I16Vec2::new(1, 2),
        packets_data: Cow::Borrowed(&data),
    };

    let bvh = Bvh::build(&mut [packet], ());

    let print = bvh.print();

    assert_eq!(print, "01	Leaf([1, 2] => [1])");

    assert_eq!(bvh.elements().len(), 1);
    assert_eq!(bvh.elements(), &[1]);

    assert_eq!(
        bvh.get_closest_slice(I16Vec2::new(1, 2)),
        Some([1].as_slice())
    );

    assert_eq!(
        bvh.get_closest_slice(I16Vec2::new(4, 4)),
        Some([1].as_slice())
    );

    assert_eq!(
        bvh.get_in(Aabb::new(I16Vec2::new(0, 0), I16Vec2::new(100, 100)))
            .len(),
        1
    );
}

#[test]
fn test_build_bvh_with_local_packet() {
    let data1 = [1, 2, 3, 4];
    let data2 = [5, 6, 7, 8];
    let data3 = [9, 10, 11, 12];
    let data4 = [13, 14, 15, 16];

    let mut input = vec![
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&data1),
        },
        ChunkWithPackets {
            location: I16Vec2::new(1, 1),
            packets_data: Cow::Borrowed(&data2),
        },
        ChunkWithPackets {
            location: I16Vec2::new(2, 2),
            packets_data: Cow::Borrowed(&data3),
        },
        ChunkWithPackets {
            location: I16Vec2::new(3, 3),
            packets_data: Cow::Borrowed(&data4),
        },
    ];

    let bvh = Bvh::build(&mut input, ());

    // Check the number of nodes in the BVH
    // assert_eq!(bvh.nodes.len(), 7);

    // Check the number of elements in the BVH
    assert_eq!(bvh.elements().len(), 16);

    // Check the contents of the elements
    assert_eq!(bvh.elements(), [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16
    ]);

    // print it out
    let s = bvh.print();

    let expected = r"
01	Internal([0, 0] -> [3, 3])
03	  Internal([2, 2] -> [3, 3])
07	    Leaf([3, 3] => [13, 14, 15, 16])
06	    Leaf([2, 2] => [9, 10, 11, 12])
02	  Internal([0, 0] -> [1, 1])
05	    Leaf([1, 1] => [5, 6, 7, 8])
04	    Leaf([0, 0] => [1, 2, 3, 4])
    "
    .trim();

    assert_eq!(s, expected);
}

#[test]
fn test_query_single_packet() {
    let data = [1, 2, 3, 4];
    let packet = ChunkWithPackets {
        location: I16Vec2::new(1, 2),
        packets_data: Cow::Borrowed(&data),
    };

    let mut input = vec![packet];
    let bvh = Bvh::build(&mut input, ());

    // Query the exact location of the packet
    let query = Aabb::new(I16Vec2::new(1, 2), I16Vec2::new(1, 2));
    let result: Vec<_> = bvh.get_in(query).into_iter().collect();
    assert_eq!(result, vec![0..4]);

    // Query a location that doesn't intersect with the packet
    let query = Aabb::new(I16Vec2::new(10, 10), I16Vec2::new(10, 10));
    let result: Vec<_> = bvh.get_in(query).into_iter().collect();
    assert_eq!(result, vec![]);
}

#[test]
fn test_query_multiple_packets() {
    let data1 = [1, 2, 3, 4];
    let data2 = [5, 6, 7, 8];
    let data3 = [9, 10, 11, 12];
    let data4 = [13, 14, 15, 16];

    let mut input = vec![
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&data1),
        },
        ChunkWithPackets {
            location: I16Vec2::new(1, 1),
            packets_data: Cow::Borrowed(&data2),
        },
        ChunkWithPackets {
            location: I16Vec2::new(2, 2),
            packets_data: Cow::Borrowed(&data3),
        },
        ChunkWithPackets {
            location: I16Vec2::new(3, 3),
            packets_data: Cow::Borrowed(&data4),
        },
    ];

    let bvh = Bvh::build(&mut input, ());

    println!("{bvh:?}");

    // Query a location that intersects with multiple packets
    let query = Aabb::new(I16Vec2::new(0, 0), I16Vec2::new(2, 2));
    let result: Vec<_> = bvh.get_in(query).into_iter().collect();
    assert_eq!(result, vec![0..12]);

    // Query a location that intersects with a single packet
    let query = Aabb::new(I16Vec2::new(3, 3), I16Vec2::new(3, 3));
    let result: Vec<_> = bvh.get_in(query).into_iter().collect();
    assert_eq!(result, vec![12..16]);

    // Query a location that doesn't intersect with any packets
    let query = Aabb::new(I16Vec2::new(10, 10), I16Vec2::new(10, 10));
    let result: Vec<_> = bvh.get_in(query).into_iter().collect();
    assert_eq!(result, vec![]);

    // we can make bytes BVH
    let _bvh = bvh.into_bytes();
}

// #[test]
// fn test_statistical_query() {
//     use std::f32::consts::PI;
//     use fastrand::Rng;
//
//     const SEED: u64 = 42;
//     const RADIUS: f32 = 100.0;
//     const NUM_PACKETS: usize = 1000;
//     const QUERY_SIZE: i16 = 50;
//     const PACKET_SIZE: usize = 4;
//     const NUM_QUERIES: usize = 100;
//
//     let mut rng = Rng::with_seed(SEED);
//     let bvh = generate_random_bvh(&mut rng, RADIUS, NUM_PACKETS, PACKET_SIZE);
//     let query_results = perform_random_queries(&bvh, &mut rng, QUERY_SIZE, NUM_QUERIES);
//
//     analyze_query_results(query_results, RADIUS, NUM_PACKETS, QUERY_SIZE);
// }
//
// fn generate_random_bvh(rng: &mut fastrand::Rng, radius: f32, num_packets: usize, packet_size: usize) -> Bvh<Vec<u8>> {
//     let mut input = Vec::with_capacity(num_packets);
//
//     for _ in 0..num_packets {
//         let r = rng.f32() * radius;
//         let theta = rng.f32() * 2.0 * std::f32::consts::PI;
//         let x = (r * theta.cos()) as i16;
//         let y = (r * theta.sin()) as i16;
//
//         let packet_data: Vec<u8> = (0..packet_size).map(|_| rng.u8(..)).collect();
//
//         input.push(ChunkWithPackets {
//             location: I16Vec2::new(x, y),
//             packets_data: Cow::Owned(packet_data),
//         });
//     }
//
//     Bvh::build(input, num_packets * packet_size)
// }
//
// fn perform_random_queries(bvh: &Bvh<Vec<u8>>, rng: &mut fastrand::Rng, query_size: i16, num_queries: usize) -> Vec<usize> {
//     (0..num_queries).map(|_| {
//         let x = rng.i16(-100..=100);
//         let y = rng.i16(-100..=100);
//         let query_region = Aabb::new(I16Vec2::new(x, y), I16Vec2::new(x + query_size, y + query_size));
//         bvh.get_in(query_region).into_iter().count()
//     }).collect()
// }
//
// fn analyze_query_results(results: Vec<usize>, radius: f32, num_packets: usize, query_size: i16) {
//     let mean = results.iter().sum::<usize>() as f32 / results.len() as f32;
//     let variance = results.iter().map(|&x| {
//         let diff = x as f32 - mean;
//         diff * diff
//     }).sum::<f32>() / results.len() as f32;
//     let std_dev = variance.sqrt();
//
//     let total_area = std::f32::consts::PI * radius * radius;
//     let query_area = query_size as f32 * query_size as f32;
//     let expected_packets = num_packets as f32 * query_area / total_area;
//
//     println!("Mean packets per query: {mean}");
//     println!("Standard deviation: {std_dev}");
//     println!("Expected packets per query: {expected_packets}");
//
//     assert!((mean - expected_packets).abs() < std_dev,
//             "Mean number of packets ({mean}) is more than one standard deviation away from expected ({expected_packets})");
// }

// Strategies for proptest
fn arb_i16vec2() -> impl Strategy<Value = I16Vec2> {
    // we can get overflow when doing the dot calculation even when converting to u16 space
    // then u32 because single 1D dot would work but 2D we can get u32::MAX + u32::MAX -> overflow
    // todo: consider some saturating dot or panicking when has values that are too large
    const MIN: i16 = i16::MIN / 2;
    const MAX: i16 = i16::MAX / 2;
    (MIN..MAX).prop_flat_map(move |x| (MIN..MAX).prop_map(move |y| I16Vec2::new(x, y)))
}

fn arb_packets_data() -> impl Strategy<Value = Vec<u8>> {
    // todo: simplification does not seem to work well for len
    prop::collection::vec(0u8..=255, ..2)
}

fn arb_chunk_with_packets() -> impl Strategy<Value = ChunkWithPackets<'static>> {
    (arb_i16vec2(), arb_packets_data()).prop_map(|(location, data)| ChunkWithPackets {
        location,
        packets_data: Cow::Owned(data),
    })
}

proptest! {
    #[test]
    fn prop_build_bvh_includes_all_elements(mut chunks in proptest::collection::vec(arb_chunk_with_packets(), 0..100)) {
        // Determine size_hint as the total number of bytes
        let bvh = Bvh::build(&mut chunks, ());

        // Verify the total number of elements matches
        let expected_len: usize = chunks.iter().map(|chunk| chunk.packets_data.len()).sum();
        assert_eq!(bvh.elements().len(), expected_len);

        // Verify all elements are present
        let mut all_elements = Vec::new();
        for chunk in &chunks {
            all_elements.extend_from_slice(&chunk.packets_data);
        }

        // the order might change so we sort
        all_elements.sort_unstable();

        let mut bvh_elements: Vec<_> = bvh.elements().to_vec();
        bvh_elements.sort_unstable();

        assert_eq!(bvh_elements, all_elements);
    }
}

fn test_query_point_returns_correct_packets(
    chunks: &mut Vec<ChunkWithPackets<'_>>,
    query_point: I16Vec2,
) {
    let bvh = Bvh::build(chunks, ());

    let result = chunks.iter().into_group_map_by(|chunk| chunk.location);

    // Find all chunks that contain the query_point
    let expected_packets: Vec<_> = result
        .into_iter()
        .min_by_key(|(pos, _)| {
            let difference = pos.as_ivec2() - query_point.as_ivec2();
            let difference = difference.abs().as_uvec2();
            difference.length_squared()
        })
        .into_iter()
        .flat_map(|(_, packets)| packets)
        .flat_map(|x| x.packets_data.clone().into_owned())
        .collect();

    // Retrieve packets from BVH
    let retrieved_packets: Vec<u8> = bvh.get_closest_slice(query_point).unwrap_or(&[]).to_vec();

    assert_eq!(
        retrieved_packets, expected_packets,
        "failed for {query_point:?}"
    );
}

proptest! {
    #[test]
    fn prop_query_point_returns_correct_packets(mut chunks in proptest::collection::vec(arb_chunk_with_packets(), 0..100), query_point in arb_i16vec2()) {
        test_query_point_returns_correct_packets(&mut chunks, query_point);
    }
}

#[test]
fn test_query_point_edge_case() {
    let mut chunks = vec![
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        },
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        },
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        },
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        },
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[0]),
        },
        ChunkWithPackets {
            location: I16Vec2::new(0, 0),
            packets_data: Cow::Borrowed(&[]),
        },
    ];
    let query_point = I16Vec2::new(0, -1);

    let bvh = Bvh::build(&mut chunks, ());

    // Find all chunks that contain the query_point

    let result = chunks.iter().into_group_map_by(|chunk| chunk.location);

    let expected_packets: Vec<_> = result
        .into_iter()
        .min_by_key(|(pos, _)| pos.as_ivec2().distance_squared(query_point.as_ivec2()))
        .into_iter()
        .flat_map(|(_, packets)| packets)
        .flat_map(|x| x.packets_data.clone().into_owned())
        .collect();

    // Retrieve packets from BVH
    let retrieved_packets: Vec<u8> = bvh.get_closest_slice(query_point).unwrap_or(&[]).to_vec();

    assert_eq!(
        retrieved_packets, expected_packets,
        "failed for {query_point:?}\n{bvh:?}"
    );
}

proptest! {
    #[test]
    fn prop_query_aabb_returns_correct_packets(
        mut chunks in proptest::collection::vec(arb_chunk_with_packets(), 0..100),
        min_x in -32768i16..32767,
        min_y in -32768i16..32767,
        max_x in -32768i16..32767,
        max_y in -32768i16..32767,
    ) {
        // Ensure min <= max for both axes
        let (min_x, max_x) = if min_x <= max_x { (min_x, max_x) } else { (max_x, min_x) };
        let (min_y, max_y) = if min_y <= max_y { (min_y, max_y) } else { (max_y, min_y) };

        let query_aabb = Aabb::new(I16Vec2::new(min_x, min_y), I16Vec2::new(max_x, max_y));

        let bvh = Bvh::build(&mut chunks, ());

        // Find all packets within the AABB
        let mut expected_packets: Vec<u8> = chunks.iter()
            .filter(|chunk| {
                let loc = chunk.location;
                loc.x >= min_x && loc.x <= max_x && loc.y >= min_y && loc.y <= max_y
            })
            .flat_map(|chunk| chunk.packets_data.iter().copied())
            .collect();

        expected_packets.sort_unstable();

        // Retrieve packets from BVH
        let mut retrieved_packets: Vec<u8> = bvh.get_in_slices(query_aabb)
            .into_iter()
            .flatten()
            .copied()
            .collect();

        retrieved_packets.sort_unstable();

        // Compare expected and retrieved packets
        assert_eq!(retrieved_packets, expected_packets);
    }
}

fn test_build_bvh_with_single_packet(packet: &ChunkWithPackets<'_>) {
    let bvh = Bvh::build(&mut [packet.clone()], ());

    // Verify the BVH has exactly one element
    assert_eq!(bvh.elements().len(), packet.packets_data.len());
    assert_eq!(bvh.elements(), packet.packets_data.as_ref());

    // The closest slice should return the packet's data
    assert_eq!(
        bvh.get_closest_slice(packet.location),
        Some(packet.packets_data.as_ref()).filter(|x| !x.is_empty())
    );

    // Querying any other point should also return the same packet
    let other_point = I16Vec2::new(packet.location.x + 10, packet.location.y + 10);
    assert_eq!(
        bvh.get_closest_slice(other_point),
        Some(packet.packets_data.as_ref()).filter(|x| !x.is_empty())
    );

    // Query within an all-encompassing AABB should include the packet
    let query_aabb = Aabb::new(
        I16Vec2::new(i16::MIN, i16::MIN),
        I16Vec2::new(i16::MAX, i16::MAX),
    );
    let retrieved_packets: Vec<u8> = bvh
        .get_in_slices(query_aabb)
        .into_iter()
        .flatten()
        .copied()
        .collect();

    assert_eq!(retrieved_packets, packet.packets_data.to_vec());
}

proptest! {
    #[test]
    fn prop_build_bvh_with_single_packet(packet in arb_chunk_with_packets()) {
        test_build_bvh_with_single_packet(&packet);
    }
}
