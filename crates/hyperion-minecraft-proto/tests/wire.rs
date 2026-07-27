//! Wire-format tests.
//!
//! The byte vectors are not invented: every one is a value the server's own
//! source pins, such as `VarInt.java`'s limits or `ServerboundHelloPacket`'s
//! 16-character name. `tests/live_server.rs` covers the other direction, by
//! handing bytes to a real 26.2 server and reading back what it sends.

use hyperion_minecraft_proto::{
    Decode, Encode, Error, Identifier, PROTOCOL_VERSION, Reader, Uuid, Writer,
    packets::{
        common::serverbound::{CookieResponse, PingRequest},
        configuration::clientbound::SelectKnownPacks,
        handshake::serverbound::Intention,
        login::{
            clientbound::{Hello as HelloRequest, LoginCompression, LoginDisconnect},
            serverbound::{Hello, LoginAcknowledged},
        },
        play, status,
        status::{
            clientbound::{PongResponse, StatusResponse},
            serverbound::StatusRequest,
        },
    },
    types::{ClientIntent, KnownPack},
};

fn encode<T: Encode>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    assert_eq!(encode(value), bytes, "encoding mismatch");
    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

// --- primitives -----------------------------------------------------------

#[test]
fn var_int_matches_vanilla_boundaries() {
    // Boundary values from VarInt.getByteSize: the byte count steps at every
    // seven bits, and -1 is the five-byte worst case.
    let cases: &[(i32, &[u8])] = &[
        (0, &[0x00]),
        (1, &[0x01]),
        (127, &[0x7F]),
        (128, &[0x80, 0x01]),
        (255, &[0xFF, 0x01]),
        (16_383, &[0xFF, 0x7F]),
        (16_384, &[0x80, 0x80, 0x01]),
        (2_097_151, &[0xFF, 0xFF, 0x7F]),
        (2_147_483_647, &[0xFF, 0xFF, 0xFF, 0xFF, 0x07]),
        (-1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        (-2_147_483_648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
    ];
    for (value, expected) in cases {
        let mut writer = Writer::new();
        writer.var_int(*value);
        assert_eq!(writer.as_slice(), *expected, "encoding {value}");

        let mut reader = Reader::new(expected);
        assert_eq!(
            reader.var_int().expect("decode"),
            *value,
            "decoding {value}"
        );
        assert!(reader.is_empty());
    }
}

#[test]
fn var_int_rejects_overlong_encoding() {
    // VarInt.read throws past five bytes rather than silently truncating.
    let bytes = [0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.var_int(), Err(Error::VarIntTooLong));
}

#[test]
fn var_long_round_trips() {
    let cases: &[(i64, &[u8])] = &[
        (0, &[0x00]),
        (127, &[0x7F]),
        (128, &[0x80, 0x01]),
        (i64::MAX, &[
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
        ]),
        (-1, &[
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
        ]),
    ];
    for (value, expected) in cases {
        let mut writer = Writer::new();
        writer.var_long(*value);
        assert_eq!(writer.as_slice(), *expected, "encoding {value}");
        let mut reader = Reader::new(expected);
        assert_eq!(reader.var_long().expect("decode"), *value);
    }
}

#[test]
fn string_prefix_counts_bytes_not_characters() {
    // Utf8String.write prefixes the *encoded byte* count while enforcing the
    // limit in UTF-16 code units, so a multi-byte string has a prefix larger
    // than its character count.
    let mut writer = Writer::new();
    writer.string("héllo").expect("encode");
    assert_eq!(writer.as_slice()[0], 6, "5 characters, 6 UTF-8 bytes");

    let mut reader = Reader::new(writer.as_slice());
    assert_eq!(reader.string().expect("decode"), "héllo");
}

#[test]
fn string_limit_is_enforced_on_both_sides() {
    let mut writer = Writer::new();
    let too_long = "x".repeat(17);
    assert_eq!(
        writer.string_with_limit(&too_long, 16),
        Err(Error::StringTooLong {
            length: 17,
            max: 16
        })
    );
}

#[test]
fn truncated_input_is_an_error_not_a_default() {
    let mut reader = Reader::new(&[0x00, 0x01]);
    assert_eq!(
        reader.i32(),
        Err(Error::UnexpectedEof {
            needed: 4,
            available: 2
        })
    );
}

// --- generated packets ----------------------------------------------------

#[test]
fn intention_round_trips() {
    let packet = Intention {
        protocol_version: PROTOCOL_VERSION,
        host_name: "localhost",
        port: 25565,
        intention: ClientIntent::Login,
    };
    // 776 -> 0x88 0x06; "localhost" -> len 9; 25565 -> 0x63 0xDD; Login -> 2.
    let expected: &[u8] = &[
        0x88, 0x06, 0x09, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', 0x63, 0xDD, 0x02,
    ];
    round_trip(&packet, expected);
}

#[test]
fn client_intent_discriminants_are_ids_not_ordinals() {
    // `ClientIntent.byId` maps 1, 2, 3; the ordinals are 0, 1, 2, and reading
    // one for the other would silently pick the wrong intent.
    let mut reader = Reader::new(&[0x00]);
    assert_eq!(
        ClientIntent::decode(&mut reader),
        Err(Error::InvalidEnum {
            name: "ClientIntent",
            value: 0
        })
    );
    round_trip(&ClientIntent::Status, &[0x01]);
    round_trip(&ClientIntent::Transfer, &[0x03]);
}

#[test]
fn status_request_is_empty() {
    round_trip(&StatusRequest, &[]);
}

#[test]
fn status_response_round_trips() {
    let json = r#"{"version":{"name":"26.2","protocol":776}}"#;
    let mut expected = vec![u8::try_from(json.len()).expect("fixture fits in one length byte")];
    expected.extend_from_slice(json.as_bytes());
    round_trip(&StatusResponse { status: json }, &expected);
}

#[test]
fn ping_and_pong_round_trip() {
    let bytes: &[u8] = &[0x00, 0x00, 0x01, 0x9A, 0x2B, 0x3C, 0x4D, 0x5E];
    round_trip(&PingRequest(0x0000_019A_2B3C_4D5E), bytes);
    round_trip(&PongResponse(0x0000_019A_2B3C_4D5E), bytes);
}

#[test]
fn one_class_serving_two_states_is_one_type() {
    // `ServerboundPingRequestPacket` is registered by both status and play, so
    // a value built for one has to be the value the other accepts.
    let ping: status::serverbound::PingRequest = play::serverbound::PingRequest(7);
    assert_eq!(ping, PingRequest(7));
}

#[test]
fn login_hello_round_trips() {
    let uuid = Uuid(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF);
    let mut expected = vec![5, b'N', b'o', b't', b'c', b'h'];
    expected.extend_from_slice(&uuid.0.to_be_bytes());
    round_trip(
        &Hello {
            name: "Notch",
            profile_id: uuid,
        },
        &expected,
    );
}

#[test]
fn login_hello_rejects_overlong_name() {
    // `ServerboundHelloPacket` writes the name with a 16-character limit, and
    // the generated struct carries that limit as `#[proto(max_len = 16)]`.
    let mut writer = Writer::new();
    let packet = Hello {
        name: "a_seventeen_char!",
        profile_id: Uuid(0),
    };
    assert!(matches!(
        packet.encode(&mut writer),
        Err(Error::StringTooLong { max: 16, .. })
    ));
}

#[test]
fn login_hello_request_round_trips() {
    let expected: &[u8] = &[
        0x00, // empty server id
        0x03, 0xAA, 0xBB, 0xCC, // public key
        0x02, 0xDE, 0xAD, // challenge
        0x01, // should_authenticate
    ];
    round_trip(
        &HelloRequest {
            server_id: "",
            public_key: &[0xAA, 0xBB, 0xCC],
            challenge: &[0xDE, 0xAD],
            should_authenticate: true,
        },
        expected,
    );
}

#[test]
fn login_compression_round_trips() {
    round_trip(&LoginCompression(256), &[0x80, 0x02]);
    // A negative threshold disables compression and needs the full five bytes.
    round_trip(&LoginCompression(-1), &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
}

#[test]
fn login_disconnect_round_trips() {
    let reason = r#"{"text":"nope"}"#;
    let mut expected = vec![u8::try_from(reason.len()).expect("fixture fits in one length byte")];
    expected.extend_from_slice(reason.as_bytes());
    round_trip(&LoginDisconnect { reason }, &expected);
}

#[test]
fn login_acknowledged_is_empty() {
    round_trip(&LoginAcknowledged, &[]);
}

#[test]
fn optional_field_is_boolean_prefixed() {
    // `ServerboundCookieResponsePacket.payload` is `Optional<byte[]>`, which
    // the derive writes as a discriminant byte and then the value.
    round_trip(
        &CookieResponse {
            key: Identifier::new("hyperion:seen").expect("valid identifier"),
            payload: Some(&[0xAB]),
        },
        &[
            0x0D, b'h', b'y', b'p', b'e', b'r', b'i', b'o', b'n', b':', b's', b'e', b'e', b'n',
            0x01, 0x01, 0xAB,
        ],
    );
    round_trip(
        &CookieResponse {
            key: Identifier::new("hyperion:seen").expect("valid identifier"),
            payload: None,
        },
        &[
            0x0D, b'h', b'y', b'p', b'e', b'r', b'i', b'o', b'n', b':', b's', b'e', b'e', b'n',
            0x00,
        ],
    );
}

#[test]
fn a_list_is_a_count_then_the_elements() {
    round_trip(
        &SelectKnownPacks {
            known_packs: vec![KnownPack {
                namespace: "minecraft",
                id: "core",
                version: "26.2",
            }],
        },
        &[
            0x01, // one pack
            0x09, b'm', b'i', b'n', b'e', b'c', b'r', b'a', b'f', b't', //
            0x04, b'c', b'o', b'r', b'e', //
            0x04, b'2', b'6', b'.', b'2',
        ],
    );
}

// --- trailing bytes -------------------------------------------------------

#[test]
fn trailing_bytes_are_rejected() {
    // A layout that is short by a field would otherwise decode "successfully"
    // and desynchronise the stream on the next packet.
    let bytes: &[u8] = &[0x80, 0x02, 0xFF];
    let mut reader = Reader::new(bytes);
    LoginCompression::decode(&mut reader).expect("decode");
    assert_eq!(reader.finish(), Err(Error::TrailingBytes(1)));
}
