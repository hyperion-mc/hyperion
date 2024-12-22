use bytes::BytesMut;
use eyre::{Context, eyre};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::info;
use valence_protocol::{
    Bounded, PacketDecoder, PacketEncoder, VarInt, WritePacket,
    packets::handshaking::handshake_c2s::HandshakeNextState,
};

/// 1.20.1
const PROTOCOL_VERSION: i32 = 763;

pub async fn launch(ip: String) -> eyre::Result<()> {
    // Connect to the TCP server
    let mut stream = TcpStream::connect(&ip)
        .await
        .wrap_err_with(|| format!("Failed to connect to {ip}"))?;

    let mut encoder = PacketEncoder::new();
    let mut decoder = PacketDecoder::new();

    // step 1: send a handshake packet
    let packet = valence_protocol::packets::handshaking::HandshakeC2s {
        protocol_version: VarInt(PROTOCOL_VERSION),
        server_address: Bounded(&ip),
        server_port: 0, // todo: probably does not matter
        next_state: HandshakeNextState::Status,
    };

    encoder
        .append_packet(&packet)
        .map_err(|e| eyre!("failed to encode handshake packet: {e}"))?;

    // status request
    let packet = valence_protocol::packets::status::QueryRequestC2s;

    encoder
        .append_packet(&packet)
        .map_err(|e| eyre!("failed to encode status request packet: {e}"))?;

    stream
        .write_all(&encoder.take())
        .await
        .wrap_err("failed to write handshake packet")?;

    // wait for response
    let mut buf = BytesMut::with_capacity(1024);

    'outer: loop {
        stream
            .read_buf(&mut buf)
            .await
            .wrap_err("failed to read query response packet")?;

        info!("read {} bytes", buf.len());

        decoder.queue_bytes(buf.split());

        if let Some(packet) = decoder
            .try_next_packet()
            .map_err(|e| eyre!("failed to decode packet: {e}"))?
        {
            tracing::info!("packet\n{packet:#?}");
            break 'outer Ok(());
        }
    }
}
