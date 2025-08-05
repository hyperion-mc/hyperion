use rkyv::{Archive, Deserialize, Serialize, with::InlineAsBox};

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct PlayerPackets<'a> {
    pub stream: u64,

    #[rkyv(with = InlineAsBox)]
    pub data: &'a [u8],
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct PlayerConnect {
    pub stream: u64,
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct PlayerDisconnect<'a> {
    pub stream: u64,
    pub reason: PlayerDisconnectReason<'a>,
}

#[derive(Archive, Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
#[non_exhaustive]
pub enum PlayerDisconnectReason<'a> {
    /// If cannot receive packets fast enough
    CouldNotKeepUp,
    LostConnection,

    Other(#[rkyv(with = InlineAsBox)] &'a str),
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct RequestSubscribeChannelPackets<'a> {
    #[rkyv(with = InlineAsBox)]
    pub channels: &'a [u32],
}

#[derive(Archive, Deserialize, Serialize, Clone, PartialEq, Debug)]
pub enum ProxyToServerMessage<'a> {
    PlayerConnect(PlayerConnect),
    PlayerDisconnect(PlayerDisconnect<'a>),
    PlayerPackets(PlayerPackets<'a>),
    RequestSubscribeChannelPackets(RequestSubscribeChannelPackets<'a>),
}
