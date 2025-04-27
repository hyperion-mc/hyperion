use rkyv::{Archive, Deserialize, Serialize, with::InlineAsBox};

use crate::ChunkPosition;

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
#[rkyv(derive(Debug))]
pub struct UpdatePlayerChunkPositions {
    pub stream: Vec<u64>,
    pub positions: Vec<ChunkPosition>,
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct SetReceiveBroadcasts {
    pub stream: u64,
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastGlobal<'a> {
    pub exclude: u64,
    pub order: u32,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastLocal<'a> {
    pub center: ChunkPosition,
    pub exclude: u64,
    pub order: u32,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct Unicast<'a> {
    pub stream: u64,
    pub order: u32,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct Flush;

/// The server must be prepared to handle other additional packets with this stream from the proxy after the server
/// sends [`Shutdown`] until the server receives [`PlayerDisconnect`] because proxy to server packets may already be
/// in transit.
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Shutdown {
    pub stream: u64,
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub enum ServerToProxyMessage<'a> {
    UpdatePlayerChunkPositions(UpdatePlayerChunkPositions),
    BroadcastGlobal(BroadcastGlobal<'a>),
    BroadcastLocal(BroadcastLocal<'a>),
    Unicast(Unicast<'a>),
    SetReceiveBroadcasts(SetReceiveBroadcasts),
    Flush(Flush),
    Shutdown(Shutdown),
}
