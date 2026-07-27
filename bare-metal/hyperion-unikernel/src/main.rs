//! Hyperion's protocol layer, on a machine with no operating system.
//!
//! This answers a Minecraft client's server-list ping using the same
//! `valence_protocol` codec the real server uses, over TCP, on whatever
//! platform it was built for. On Linux that is an ordinary process. On
//! `x86_64-unknown-hermit` it is a unikernel image: the binary *is* the kernel,
//! it boots under QEMU, brings up virtio-net, takes a DHCP lease, and listens.
//!
//! It is a demonstration, not a server. What it proves is narrow and specific:
//! that the wire protocol, the codec and the platform seam all work with no OS
//! underneath, which is the part of hyperion nobody had established. It does
//! not log anyone in — see `docs/bare-metal.md` for why the rest of the server
//! does not build yet.

// Linking the kernel is a side effect of the dependency existing; nothing calls
// into it directly.
#[cfg(target_os = "hermit")]
use hermit as _;

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
};

use anyhow::{Context, bail, ensure};
use hyperion_platform::{CAPABILITIES, clock, net, parallelism};
use valence_bytes::CowUtf8Bytes;
use valence_protocol::{
    DecodeBytes, Encode, MAX_PACKET_SIZE, PROTOCOL_VERSION, Packet, PacketEncoder,
    bytes::Bytes,
    packets::{
        handshaking::{HandshakeC2s, handshake_c2s::HandshakeNextState},
        status::{QueryPingC2s, QueryPongS2c, QueryRequestC2s, QueryResponseS2c},
    },
};

/// Where to listen unless `HYPERION_PORT` says otherwise.
///
/// The environment is the only configuration channel that works on both
/// platforms: a unikernel has no config file to read, but Hermit does populate
/// `std::env` from the boot command line.
const DEFAULT_PORT: u16 = 25565;

fn port() -> u16 {
    std::env::var("HYPERION_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn main() -> anyhow::Result<()> {
    let started = clock::monotonic();

    println!("[hyperion] platform: {}", hyperion_platform::NAME);
    println!("[hyperion] capabilities: {CAPABILITIES:?}");
    println!("[hyperion] parallelism: {}", parallelism::available());
    match clock::wall_clock() {
        Some(t) => println!("[hyperion] wall clock: {t:?}"),
        None => println!("[hyperion] wall clock: unavailable on this platform"),
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], port()));
    let listener = net::bind_tcp(addr)?;
    println!(
        "[hyperion] listening on {addr} after {elapsed:?}",
        elapsed = started.elapsed()
    );

    for stream in listener.incoming() {
        let stream = stream.context("accept failed")?;
        let peer = stream.peer_addr().ok();
        // One connection at a time. A status ping is three packets and
        // concurrency is not the thing under test here.
        if let Err(e) = serve(stream) {
            println!("[hyperion] {peer:?}: {e:#}");
        }
    }

    Ok(())
}

/// Handshake, status, ping. The whole server-list exchange.
fn serve(mut stream: TcpStream) -> anyhow::Result<()> {
    println!("[hyperion] accepted {peer:?}", peer = stream.peer_addr());

    let handshake: HandshakeC2s<'_> = read_packet(&mut stream)?;
    println!(
        "[hyperion] handshake: protocol={protocol} host={host} port={port} next={next:?}",
        protocol = handshake.protocol_version.0,
        host = handshake.server_address.0.as_str(),
        port = handshake.server_port,
        next = handshake.next_state
    );

    ensure!(
        handshake.next_state == HandshakeNextState::Status,
        "only the status handshake is implemented on this platform"
    );

    let _: QueryRequestC2s = read_packet(&mut stream)?;

    let json = status_json();
    write_packet(
        &mut stream,
        &QueryResponseS2c {
            json: CowUtf8Bytes::Borrowed(&json),
        },
    )?;
    println!("[hyperion] sent status");

    // A client measures latency from this round trip. Echoing the payload
    // unchanged is the whole protocol.
    let ping: QueryPingC2s = read_packet(&mut stream)?;
    write_packet(
        &mut stream,
        &QueryPongS2c {
            payload: ping.payload,
        },
    )?;
    println!("[hyperion] ponged {payload:#x}", payload = ping.payload);

    Ok(())
}

/// The server-list entry a client renders.
fn status_json() -> String {
    format!(
        r#"{{"version":{{"name":"hyperion/{name}","protocol":{PROTOCOL_VERSION}}},"players":{{"max":10000,"online":0,"sample":[]}},"description":{{"text":"hyperion on {name}, no operating system"}}}}"#,
        name = hyperion_platform::NAME
    )
}

/// Read one length-prefixed frame and decode it as `P`.
///
/// `valence_protocol` keeps its framing decoder private and hyperion's own
/// lives in a crate that does not build for this target yet, so the framing is
/// open-coded here. It is only correct because compression is off: with a
/// threshold set, the body carries a second length that this does not read.
fn read_packet<P>(stream: &mut TcpStream) -> anyhow::Result<P>
where
    P: Packet + DecodeBytes,
{
    let len = read_var_int(stream).context("packet length")?;
    ensure!(
        (0..=MAX_PACKET_SIZE).contains(&len),
        "packet length {len} out of bounds"
    );

    let mut body = vec![0u8; usize::try_from(len).expect("length is non-negative")];
    stream.read_exact(&mut body).context("packet body")?;
    let mut body = Bytes::from(body);

    let id = read_var_int(&mut body.as_ref()).context("packet id")?;
    let id_len = var_int_len(id);
    let _ = body.split_to(id_len);

    ensure!(
        id == P::ID,
        "expected {name} (id {expected}), got id {id}",
        name = P::NAME,
        expected = P::ID
    );

    let packet = P::decode_bytes(&mut body).context("decode failed")?;
    ensure!(
        body.is_empty(),
        "{name}: {left} trailing bytes",
        name = P::NAME,
        left = body.len()
    );
    Ok(packet)
}

/// Read a Minecraft `VarInt`: seven bits per byte, little end first, high bit
/// meaning "another byte follows", five bytes maximum.
fn read_var_int(r: &mut impl Read) -> anyhow::Result<i32> {
    let mut value = 0i32;
    for shift in 0..5 {
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte).context("short read")?;
        value |= i32::from(byte[0] & 0x7F) << (shift * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("`VarInt` longer than five bytes")
}

/// How many bytes the wire form of `value` occupies.
const fn var_int_len(value: i32) -> usize {
    let bits = 32 - (value.cast_unsigned() | 1).leading_zeros();
    (bits as usize).div_ceil(7)
}

fn write_packet<P>(stream: &mut TcpStream, packet: &P) -> anyhow::Result<()>
where
    P: Packet + Encode,
{
    let mut encoder = PacketEncoder::new();
    encoder.append_packet(packet)?;
    stream.write_all(&encoder.take()).context("write failed")?;
    stream.flush().context("flush failed")
}
