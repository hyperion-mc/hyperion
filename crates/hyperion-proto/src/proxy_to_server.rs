use rkyv::{Archive, Deserialize, Serialize, with::InlineAsBox};

/// Packets sent from a player to be forwarded to the server
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct PlayerPackets<'a> {
    /// Stream ID for the player connection
    pub stream: u64,

    /// Raw packet data bytes
    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

/// Message sent when a player connects to the proxy
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct PlayerConnect {
    /// Stream ID for the new player connection
    pub stream: u64,
}

/// Message sent when a player disconnects from the proxy
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct PlayerDisconnect<'a> {
    /// Stream ID for the disconnected player
    pub stream: u64,
    /// Reason for the disconnection
    pub reason: PlayerDisconnectReason<'a>,
}

/// Reason for a player disconnection
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
#[non_exhaustive]
pub enum PlayerDisconnectReason<'a> {
    /// If cannot receive packets fast enough
    CouldNotKeepUp,
    /// Connection was lost
    LostConnection,
    /// Other disconnection reason with custom message
    Other(#[rkyv(with = InlineAsBox)] &'a str),
}

/// Messages sent from the proxy to the server
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq, Debug)]
pub enum ProxyToServerMessage<'a> {
    /// A new player has connected
    PlayerConnect(PlayerConnect),
    /// A player has disconnected
    PlayerDisconnect(PlayerDisconnect<'a>),
    /// Packets received from a player
    PlayerPackets(PlayerPackets<'a>),
}
