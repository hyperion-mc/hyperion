use hyperion_proto::{ChunkPosition, ServerToProxyMessage, UpdateChannelPosition};

use crate::net::{ConnectionId, ProxyId};

#[derive(Clone, PartialEq)]
pub struct UpdatePlayerPosition {
    pub stream: ConnectionId,
    pub position: ChunkPosition,
}

#[derive(Clone, PartialEq)]
pub struct UpdatePlayerPositions<'a> {
    pub updates: &'a [UpdatePlayerPosition],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AddChannel<'a> {
    pub channel_id: u32,

    pub unsubscribe_packets: &'a [u8],
}

#[derive(Clone, PartialEq)]
pub struct UpdateChannelPositions<'a> {
    pub updates: &'a [UpdateChannelPosition],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RemoveChannel {
    pub channel_id: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubscribeChannelPackets<'a> {
    pub channel_id: u32,
    pub receiver: ProxyId,
    pub exclude: Option<ConnectionId>,

    pub data: &'a [u8],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SetReceiveBroadcasts {
    pub stream: ConnectionId,
}

#[derive(Clone, PartialEq, Eq)]
pub struct BroadcastGlobal<'a> {
    pub exclude: Option<ConnectionId>,

    pub data: &'a [u8],
}

#[derive(Clone, PartialEq)]
pub struct BroadcastLocal<'a> {
    pub center: ChunkPosition,
    pub exclude: Option<ConnectionId>,

    pub data: &'a [u8],
}

#[derive(Clone, PartialEq, Eq)]
pub struct BroadcastChannel<'a> {
    pub channel_id: u32,
    pub exclude: Option<ConnectionId>,

    pub data: &'a [u8],
}

#[derive(Clone, PartialEq, Eq)]
pub struct Unicast<'a> {
    pub stream: ConnectionId,

    pub data: &'a [u8],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shutdown {
    pub stream: ConnectionId,
}

#[derive(Clone, PartialEq)]
pub enum IntermediateServerToProxyMessage<'a> {
    UpdatePlayerPositions(UpdatePlayerPositions<'a>),
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

impl IntermediateServerToProxyMessage<'_> {
    /// Whether the result of [`IntermediateServerToProxyMessage::transform_for_proxy`] will be
    /// affected by the proxy id provided
    #[must_use]
    pub const fn affected_by_proxy(&self) -> bool {
        match self {
            Self::UpdatePlayerPositions(_)
            | Self::SubscribeChannelPackets(_)
            | Self::BroadcastGlobal(_)
            | Self::BroadcastLocal(_)
            | Self::BroadcastChannel(_)
            | Self::Unicast(_)
            | Self::SetReceiveBroadcasts(_)
            | Self::Shutdown(_) => true,
            Self::AddChannel(_) | Self::UpdateChannelPositions(_) | Self::RemoveChannel(_) => false,
        }
    }

    /// Transforms an intermediate message to a message suitable for sending to a particular proxy.
    /// Returns `None` if this message should not be sent to the proxy.
    #[must_use]
    pub fn transform_for_proxy(&self, proxy_id: ProxyId) -> Option<ServerToProxyMessage<'_>> {
        let filter_map_connection_id =
            |id: ConnectionId| (id.proxy_id() == proxy_id).then(|| id.inner());
        match self {
            Self::UpdatePlayerPositions(message) => {
                Some(ServerToProxyMessage::UpdatePlayerPositions(
                    hyperion_proto::UpdatePlayerPositions {
                        updates: message
                            .updates
                            .iter()
                            .filter_map(|update| {
                                Some(hyperion_proto::UpdatePlayerPosition {
                                    stream: filter_map_connection_id(update.stream)?,
                                    position: update.position,
                                })
                            })
                            .collect::<Vec<_>>(),
                    },
                ))
            }
            Self::AddChannel(message) => Some(ServerToProxyMessage::AddChannel(
                hyperion_proto::AddChannel {
                    channel_id: message.channel_id,
                    unsubscribe_packets: message.unsubscribe_packets,
                },
            )),
            Self::UpdateChannelPositions(message) => {
                Some(ServerToProxyMessage::UpdateChannelPositions(
                    hyperion_proto::UpdateChannelPositions {
                        updates: message.updates,
                    },
                ))
            }
            Self::RemoveChannel(message) => Some(ServerToProxyMessage::RemoveChannel(
                hyperion_proto::RemoveChannel {
                    channel_id: message.channel_id,
                },
            )),
            Self::SubscribeChannelPackets(message) => (message.receiver == proxy_id).then(|| {
                ServerToProxyMessage::SubscribeChannelPackets(
                    hyperion_proto::SubscribeChannelPackets {
                        channel_id: message.channel_id,
                        exclude: message
                            .exclude
                            .and_then(filter_map_connection_id)
                            .unwrap_or_default(),
                        data: message.data,
                    },
                )
            }),
            Self::BroadcastGlobal(message) => Some(ServerToProxyMessage::BroadcastGlobal(
                hyperion_proto::BroadcastGlobal {
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::BroadcastLocal(message) => Some(ServerToProxyMessage::BroadcastLocal(
                hyperion_proto::BroadcastLocal {
                    center: message.center,
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::BroadcastChannel(message) => Some(ServerToProxyMessage::BroadcastChannel(
                hyperion_proto::BroadcastChannel {
                    channel_id: message.channel_id,
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::Unicast(message) => {
                Some(ServerToProxyMessage::Unicast(hyperion_proto::Unicast {
                    stream: filter_map_connection_id(message.stream)?,
                    data: message.data,
                }))
            }
            Self::SetReceiveBroadcasts(message) => Some(
                ServerToProxyMessage::SetReceiveBroadcasts(hyperion_proto::SetReceiveBroadcasts {
                    stream: filter_map_connection_id(message.stream)?,
                }),
            ),
            Self::Shutdown(message) => Some(ServerToProxyMessage::SetReceiveBroadcasts(
                hyperion_proto::SetReceiveBroadcasts {
                    stream: filter_map_connection_id(message.stream)?,
                },
            )),
        }
    }
}
