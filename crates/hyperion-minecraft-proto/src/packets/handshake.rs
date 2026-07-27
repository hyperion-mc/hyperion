//! Handshake state. One packet, sent by the client before anything else.

use crate::{Decode, Encode, Error, Reader, Result, Writer};

/// What the client wants to do after the handshake.
///
/// Discriminants are from `net.minecraft.network.protocol.handshake.ClientIntent`.
/// Note they start at one, not zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ClientIntent {
    /// Query the server list ping.
    Status = 1,
    /// Log in and play.
    Login = 2,
    /// Arrive via a transfer from another server.
    Transfer = 3,
}

impl ClientIntent {
    /// Resolve a wire discriminant.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] for anything outside 1..=3, matching
    /// `ClientIntent.byId` throwing on an unknown id.
    pub const fn from_raw(value: i32) -> Result<Self> {
        match value {
            1 => Ok(Self::Status),
            2 => Ok(Self::Login),
            3 => Ok(Self::Transfer),
            _ => Err(Error::InvalidEnum {
                name: "ClientIntent",
                value,
            }),
        }
    }

    /// The wire discriminant.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self as i32
    }
}

/// `minecraft:intention`, id 0 (`ClientIntentionPacket`).
///
/// The one packet whose layout is frozen across protocol versions, because a
/// server has to read `protocol_version` before it knows which version it is
/// speaking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intention<'a> {
    /// Protocol number the client speaks.
    pub protocol_version: i32,
    /// Host name the client connected to, as typed.
    pub host_name: &'a str,
    /// Port the client connected to.
    pub port: u16,
    /// What the client intends to do next.
    pub intention: ClientIntent,
}

/// `ClientIntentionPacket.MAX_HOST_LENGTH`.
const MAX_HOST_LENGTH: usize = 255;

impl Encode for Intention<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.protocol_version);
        // The server writes without a limit here and enforces it on read; the
        // limit is applied on both sides so a malformed value cannot leave.
        writer.string_with_limit(self.host_name, MAX_HOST_LENGTH)?;
        writer.u16(self.port);
        writer.var_int(self.intention.to_raw());
        Ok(())
    }
}

impl<'a> Decode<'a> for Intention<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            protocol_version: reader.var_int()?,
            host_name: reader.string_with_limit(MAX_HOST_LENGTH)?,
            port: reader.u16()?,
            intention: ClientIntent::from_raw(reader.var_int()?)?,
        })
    }
}
