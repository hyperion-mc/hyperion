//! Status state: the server list ping exchange.

use crate::{Decode, Encode, Reader, Result, Writer};

/// `minecraft:status_request`, serverbound id 0 (`ServerboundStatusRequestPacket`).
///
/// Empty on the wire; the server's codec is `StreamCodec.unit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusRequest;

impl Encode for StatusRequest {
    fn encode(&self, _writer: &mut Writer) -> Result<()> {
        Ok(())
    }
}

impl Decode<'_> for StatusRequest {
    fn decode(_reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self)
    }
}

/// `minecraft:status_response`, clientbound id 0 (`ClientboundStatusResponsePacket`).
///
/// The body is a JSON document capped at `Short.MAX_VALUE` characters. The
/// server parses it into `ServerStatus` with a DFU codec, but that shapes only
/// the in-memory value: on the wire it is a length-prefixed string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse<'a> {
    /// Server status as a JSON document.
    pub status: &'a str,
}

impl Encode for StatusResponse<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.status)
    }
}

impl<'a> Decode<'a> for StatusResponse<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            status: reader.string()?,
        })
    }
}

/// `minecraft:ping_request`, serverbound id 1 (`ServerboundPingRequestPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingRequest {
    /// Opaque value echoed back in the pong.
    pub time: i64,
}

impl Encode for PingRequest {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.time);
        Ok(())
    }
}

impl Decode<'_> for PingRequest {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            time: reader.i64()?,
        })
    }
}

/// `minecraft:pong_response`, clientbound id 1 (`ClientboundPongResponsePacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PongResponse {
    /// The value from the matching ping.
    pub time: i64,
}

impl Encode for PongResponse {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.time);
        Ok(())
    }
}

impl Decode<'_> for PongResponse {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            time: reader.i64()?,
        })
    }
}
