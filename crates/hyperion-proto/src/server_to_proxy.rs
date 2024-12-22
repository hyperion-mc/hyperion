use rkyv::{Archive, Deserialize, Serialize, with::InlineAsBox};

use crate::ChunkPosition;

/// Updates the chunk positions that players are currently in
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
#[rkyv(derive(Debug))]
pub struct UpdatePlayerChunkPositions {
    /// Stream IDs for the players to update
    pub stream: Vec<u64>,
    /// New chunk positions for each player
    pub positions: Vec<ChunkPosition>,
}

/// Sets whether a player should receive broadcast messages
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct SetReceiveBroadcasts {
    /// Stream ID of the player
    pub stream: u64,
}

/// Broadcasts a packet to all connected players except one
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastGlobal<'a> {
    /// Stream ID of the player to exclude
    pub exclude: u64,
    /// Order number for packet sequencing
    pub order: u32,

    /// Raw packet data bytes
    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

/// Broadcasts a packet to players near a chunk position
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastLocal<'a> {
    /// Center chunk position for the broadcast
    pub center: ChunkPosition,
    /// Stream ID of the player to exclude
    pub exclude: u64,
    /// Order number for packet sequencing
    pub order: u32,

    /// Raw packet data bytes
    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

/// Sends a packet to a specific player
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct Unicast<'a> {
    /// Stream ID of the target player
    pub stream: u64,
    /// Order number for packet sequencing
    pub order: u32,

    /// Raw packet data bytes
    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

/// Flushes pending packets to clients
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct Flush;

/// Messages sent from the server to the proxy
#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub enum ServerToProxyMessage<'a> {
    /// Update player chunk positions
    UpdatePlayerChunkPositions(UpdatePlayerChunkPositions),
    /// Broadcast a packet globally
    BroadcastGlobal(BroadcastGlobal<'a>),
    /// Broadcast a packet locally around a position
    BroadcastLocal(BroadcastLocal<'a>),
    /// Send a packet to a specific player
    Unicast(Unicast<'a>),
    /// Set whether a player receives broadcasts
    SetReceiveBroadcasts(SetReceiveBroadcasts),
    /// Flush pending packets
    Flush(Flush),
}
