//! Login state: authentication and the handover into configuration.

use crate::{Decode, Encode, Reader, Result, Writer};

/// `minecraft:hello`, serverbound id 0 (`ServerboundHelloPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello<'a> {
    /// Player name, capped at 16 characters by the server.
    pub name: &'a str,
    /// Offline-mode or cached profile id.
    pub profile_id: u128,
}

/// `ServerboundHelloPacket` reads the name with a 16-character limit.
const MAX_NAME_LENGTH: usize = 16;

impl Encode for Hello<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string_with_limit(self.name, MAX_NAME_LENGTH)?;
        writer.uuid(self.profile_id);
        Ok(())
    }
}

impl<'a> Decode<'a> for Hello<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            name: reader.string_with_limit(MAX_NAME_LENGTH)?,
            profile_id: reader.uuid()?,
        })
    }
}

/// `minecraft:hello`, clientbound id 1 (`ClientboundHelloPacket`).
///
/// Carries the encryption request. `should_authenticate` was added when Mojang
/// split online-mode enforcement from the presence of encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloRequest<'a> {
    /// Server id string, capped at 20 characters.
    pub server_id: &'a str,
    /// DER-encoded public key.
    pub public_key: &'a [u8],
    /// Verify token the client must return encrypted.
    pub challenge: &'a [u8],
    /// Whether the client should complete the session-server handshake.
    pub should_authenticate: bool,
}

/// `ClientboundHelloPacket` reads the server id with a 20-character limit.
const MAX_SERVER_ID_LENGTH: usize = 20;

impl Encode for HelloRequest<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string_with_limit(self.server_id, MAX_SERVER_ID_LENGTH)?;
        writer.byte_array(self.public_key)?;
        writer.byte_array(self.challenge)?;
        writer.bool(self.should_authenticate);
        Ok(())
    }
}

impl<'a> Decode<'a> for HelloRequest<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            server_id: reader.string_with_limit(MAX_SERVER_ID_LENGTH)?,
            public_key: reader.byte_array()?,
            challenge: reader.byte_array()?,
            should_authenticate: reader.bool()?,
        })
    }
}

/// `minecraft:key`, serverbound id 1 (`ServerboundKeyPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key<'a> {
    /// Shared secret, encrypted with the server's public key.
    pub key_bytes: &'a [u8],
    /// The challenge from the hello, encrypted the same way.
    pub encrypted_challenge: &'a [u8],
}

impl Encode for Key<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.byte_array(self.key_bytes)?;
        writer.byte_array(self.encrypted_challenge)?;
        Ok(())
    }
}

impl<'a> Decode<'a> for Key<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            key_bytes: reader.byte_array()?,
            encrypted_challenge: reader.byte_array()?,
        })
    }
}

/// `minecraft:login_compression`, clientbound id 3 (`ClientboundLoginCompressionPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginCompression {
    /// Packets at or above this size are compressed. Negative disables it.
    pub compression_threshold: i32,
}

impl Encode for LoginCompression {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.compression_threshold);
        Ok(())
    }
}

impl Decode<'_> for LoginCompression {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            compression_threshold: reader.var_int()?,
        })
    }
}

/// `minecraft:login_disconnect`, clientbound id 0 (`ClientboundLoginDisconnectPacket`).
///
/// The reason is JSON rather than the NBT-encoded component used after login,
/// because at this point no registries have been sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDisconnect<'a> {
    /// Disconnect reason, as a JSON chat component.
    pub reason: &'a str,
}

/// `ByteBufCodecs.lenientJson(262144)` in `ClientboundLoginDisconnectPacket`.
const MAX_REASON_LENGTH: usize = 262_144;

impl Encode for LoginDisconnect<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string_with_limit(self.reason, MAX_REASON_LENGTH)
    }
}

impl<'a> Decode<'a> for LoginDisconnect<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            reason: reader.string_with_limit(MAX_REASON_LENGTH)?,
        })
    }
}

/// `minecraft:login_acknowledged`, serverbound id 3 (`ServerboundLoginAcknowledgedPacket`).
///
/// Empty on the wire; moves the connection into the configuration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoginAcknowledged;

impl Encode for LoginAcknowledged {
    fn encode(&self, _writer: &mut Writer) -> Result<()> {
        Ok(())
    }
}

impl Decode<'_> for LoginAcknowledged {
    fn decode(_reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self)
    }
}

/// `minecraft:cookie_request`, clientbound id 5 (`ClientboundCookieRequestPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieRequest<'a> {
    /// Identifier of the cookie being requested.
    pub key: &'a str,
}

impl Encode for CookieRequest<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.key)
    }
}

impl<'a> Decode<'a> for CookieRequest<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            key: reader.string()?,
        })
    }
}
