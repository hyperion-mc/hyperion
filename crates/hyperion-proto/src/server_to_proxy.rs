use rkyv::{Archive, Deserialize, Serialize, with::InlineAsBox};

use crate::ChunkPosition;

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
#[rkyv(derive(Debug))]
pub struct UpdatePlayerPositions {
    pub stream: Vec<u64>,
    pub positions: Vec<ChunkPosition>,
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
pub struct AddChannel<'a> {
    pub channel_id: u32,

    #[rkyv(with = InlineAsBox)]
    pub unsubscribe_packets: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct UpdateChannelPosition {
    pub channel_id: u32,
    pub position: ChunkPosition,
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct UpdateChannelPositions<'a> {
    #[rkyv(with = InlineAsBox)]
    pub updates: &'a [UpdateChannelPosition],
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct RemoveChannel {
    pub channel_id: u32,
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
pub struct SubscribeChannelPackets<'a> {
    pub channel_id: u32,
    pub exclude: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[rkyv(derive(Debug))]
pub struct SetReceiveBroadcasts {
    pub stream: u64,
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastGlobal<'a> {
    pub exclude: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastLocal<'a> {
    pub center: ChunkPosition,
    pub exclude: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct BroadcastChannel<'a> {
    pub channel_id: u32,
    pub exclude: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub struct Unicast<'a> {
    pub stream: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

/// The server must be prepared to handle other additional packets with this stream from the proxy after the server
/// sends [`Shutdown`] until the server receives [`crate::PlayerDisconnect`] because proxy to server packets may
/// already be in transit.
#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct Shutdown {
    pub stream: u64,
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq)]
pub enum ServerToProxyMessage<'a> {
    UpdatePlayerPositions(UpdatePlayerPositions),
    AddChannel(AddChannel<'a>),
    UpdateChannelPositions(UpdateChannelPositions<'a>),
    RemoveChannel(RemoveChannel),
    SubscribeChannelPackets(SubscribeChannelPackets<'a>),
    BroadcastGlobal(BroadcastGlobal<'a>),
    BroadcastLocal(BroadcastLocal<'a>),
    BroadcastChannel(BroadcastChannel<'a>),
    Unicast(Unicast<'a>),
    SetReceiveBroadcasts(SetReceiveBroadcasts),
    Shutdown(Shutdown),
}
