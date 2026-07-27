//! Round-trip against a real Minecraft server.
//!
//! Ignored by default because it needs a server listening. Point it at one and
//! run it with:
//!
//! ```text
//! HYPERION_MC_SERVER=127.0.0.1:25565 cargo test -p hyperion-minecraft-proto \
//!     --test live_server -- --ignored --nocapture
//! ```
//!
//! This is the check that a synthetic round-trip cannot make: encode with our
//! codec, hand the bytes to Mojang's decoder, and decode what Mojang's encoder
//! sends back.

use std::{
    env,
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use hyperion_minecraft_proto::{
    Decode, Encode, PROTOCOL_VERSION, Reader, VarInt, Writer,
    packets::{
        handshake::serverbound::Intention,
        status::{
            clientbound::{PongResponse, StatusResponse},
            serverbound::{PingRequest, StatusRequest},
        },
    },
    types::ClientIntent,
};

/// Wrap a packet body in the length-prefixed frame the server expects.
///
/// Framing is `VarInt(len) | VarInt(packet id) | body`, with no compression
/// until the server sends a compression threshold, which it never does before
/// login.
fn frame<T: Encode>(packet_id: i32, packet: &T) -> Vec<u8> {
    let mut body = Writer::new();
    body.var_int(packet_id);
    packet.encode(&mut body).expect("encode body");
    let body = body.into_vec();

    let mut out = Writer::new();
    out.var_int(i32::try_from(body.len()).expect("body fits in an i32"));
    out.raw(&body);
    out.into_vec()
}

/// Read one frame, returning `(packet id, body)`.
fn read_frame(stream: &mut TcpStream) -> (i32, Vec<u8>) {
    // The length prefix is a VarInt, so it has to be read a byte at a time:
    // its own length is not known until the terminator byte arrives.
    let mut length_bytes = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).expect("read length byte");
        length_bytes.push(byte[0]);
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    let length = Reader::new(&length_bytes)
        .var_int()
        .expect("decode frame length");

    let mut body = vec![0u8; usize::try_from(length).expect("non-negative length")];
    stream.read_exact(&mut body).expect("read frame body");

    let mut reader = Reader::new(&body);
    let packet_id = reader.var_int().expect("decode packet id");
    let consumed = body.len() - reader.remaining_len();
    (packet_id, body[consumed..].to_vec())
}

#[test]
#[ignore = "needs a running Minecraft server; set HYPERION_MC_SERVER"]
fn status_ping_against_real_server() {
    let address = env::var("HYPERION_MC_SERVER").expect("HYPERION_MC_SERVER must be set");
    let (host, port) = address.rsplit_once(':').expect("address must be host:port");
    let port: i16 = port.parse().expect("port must be a number");

    let mut stream = TcpStream::connect(&address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");

    // Handshake, then status request, in one write the way a real client does.
    let handshake = Intention {
        protocol_version: VarInt(PROTOCOL_VERSION),
        host_name: host,
        port,
        intention: ClientIntent::Status,
    };
    stream
        .write_all(&frame(0, &handshake))
        .expect("send handshake");
    stream
        .write_all(&frame(0, &StatusRequest))
        .expect("send status request");

    let (packet_id, body) = read_frame(&mut stream);
    assert_eq!(packet_id, 0, "status_response has clientbound id 0");

    let mut reader = Reader::new(&body);
    let response = StatusResponse::decode(&mut reader).expect("decode status response");
    reader.finish().expect("status response fully consumed");

    eprintln!(
        "status response ({} bytes): {}",
        body.len(),
        response.status
    );

    // The server reports the protocol it speaks; it must be the one we generated
    // our tables from, or the whole pipeline is pointed at the wrong version.
    let expected = format!(r#""protocol":{PROTOCOL_VERSION}"#);
    assert!(
        response.status.replace(' ', "").contains(&expected),
        "server did not report protocol {PROTOCOL_VERSION}: {}",
        response.status
    );

    // Ping/pong: the server echoes the value back unchanged, so a mismatch here
    // is a big-endian i64 bug on one side or the other.
    let nonce = 0x0123_4567_89AB_CDEF_i64;
    stream
        .write_all(&frame(1, &PingRequest(nonce)))
        .expect("send ping");

    let (packet_id, body) = read_frame(&mut stream);
    assert_eq!(packet_id, 1, "pong_response has clientbound id 1");
    let mut reader = Reader::new(&body);
    let pong = PongResponse::decode(&mut reader).expect("decode pong");
    reader.finish().expect("pong fully consumed");
    assert_eq!(pong.0, nonce, "server echoed a different value");

    eprintln!("pong echoed {:#x} correctly", pong.0);
}
