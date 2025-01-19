use antithesis_sdk::serde_json::json;
use bytes::BytesMut;
use eyre::{Context, eyre};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::info;
use valence_protocol::{
    Bounded, PacketDecoder, PacketEncoder, VarInt,
    packets::handshaking::handshake_c2s::HandshakeNextState,
};

/// 1.20.1
const PROTOCOL_VERSION: i32 = 763;

#[derive(Debug, PartialEq, Eq)]
pub struct ServerAddress {
    host: String,
    port: u16,
}

fn parse_address(address: &str) -> eyre::Result<ServerAddress> {
    static ADDRESS_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?P<host>[^:]+)(?::(?P<port>\d+))?$").unwrap()
    });

    let captures = ADDRESS_REGEX
        .captures(address)
        .ok_or_else(|| eyre!("Invalid address format"))?;

    let host = captures
        .name("host")
        .ok_or_else(|| eyre!("Missing host in address"))?
        .as_str()
        .to_owned();

    let port = captures
        .name("port")
        .and_then(|m| m.as_str().parse::<u16>().ok())
        .unwrap_or(25565);

    Ok(ServerAddress { host, port })
}

pub async fn launch(address: &str) -> eyre::Result<()> {
    let server_addr = parse_address(address)?;

    // Connect to the TCP server
    let mut stream = TcpStream::connect(address)
        .await
        .wrap_err_with(|| format!("Failed to connect to {address}"))?;

    antithesis_sdk::assert_reachable!("connected to the Minecraft server");

    let mut encoder = PacketEncoder::new();
    let mut decoder = PacketDecoder::new();

    // step 1: send a handshake packet
    let packet = valence_protocol::packets::handshaking::HandshakeC2s {
        protocol_version: VarInt(PROTOCOL_VERSION),
        server_address: Bounded(&server_addr.host),
        server_port: server_addr.port,
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
            let packet = format!("{packet:?}");
            let packet = json! ({
                "value": packet,
            });

            antithesis_sdk::assert_reachable!("received a packet", &packet);

            info!("packet\n{packet:#?}");
            break 'outer Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_with_port() {
        let result = parse_address("example.com:12345").unwrap();
        assert_eq!(result, ServerAddress {
            host: "example.com".to_string(),
            port: 12345
        });
    }

    #[test]
    fn test_parse_address_without_port() {
        let result = parse_address("example.com").unwrap();
        assert_eq!(result, ServerAddress {
            host: "example.com".to_string(),
            port: 25565
        });
    }

    #[test]
    fn test_parse_address_invalid() {
        assert!(parse_address("").is_err());
        assert!(parse_address(":1234").is_err());
        assert!(parse_address("example.com:").is_err());
    }
}
