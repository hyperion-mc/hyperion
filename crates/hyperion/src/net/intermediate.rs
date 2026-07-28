use hyperion_proxy_proto::{ChunkPosition, ServerToProxyMessage, UpdateChannelPosition};

use crate::net::{ConnectionId, ProxyId};

#[derive(Clone, PartialEq)]
pub struct UpdatePlayerPositions {
    pub stream: Vec<ConnectionId>,
    pub positions: Vec<ChunkPosition>,
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
                    hyperion_proxy_proto::UpdatePlayerPositions {
                        stream: message
                            .stream
                            .iter()
                            .copied()
                            .filter_map(filter_map_connection_id)
                            .collect::<Vec<_>>(),
                        positions: message.positions.clone(),
                    },
                ))
            }
            Self::AddChannel(message) => Some(ServerToProxyMessage::AddChannel(
                hyperion_proxy_proto::AddChannel {
                    channel_id: message.channel_id,
                    unsubscribe_packets: message.unsubscribe_packets,
                },
            )),
            Self::UpdateChannelPositions(message) => {
                Some(ServerToProxyMessage::UpdateChannelPositions(
                    hyperion_proxy_proto::UpdateChannelPositions {
                        updates: message.updates,
                    },
                ))
            }
            Self::RemoveChannel(message) => Some(ServerToProxyMessage::RemoveChannel(
                hyperion_proxy_proto::RemoveChannel {
                    channel_id: message.channel_id,
                },
            )),
            Self::SubscribeChannelPackets(message) => {
                Some(ServerToProxyMessage::SubscribeChannelPackets(
                    hyperion_proxy_proto::SubscribeChannelPackets {
                        channel_id: message.channel_id,
                        exclude: message
                            .exclude
                            .and_then(filter_map_connection_id)
                            .unwrap_or_default(),
                        data: message.data,
                    },
                ))
            }
            Self::BroadcastGlobal(message) => Some(ServerToProxyMessage::BroadcastGlobal(
                hyperion_proxy_proto::BroadcastGlobal {
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::BroadcastLocal(message) => Some(ServerToProxyMessage::BroadcastLocal(
                hyperion_proxy_proto::BroadcastLocal {
                    center: message.center,
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::BroadcastChannel(message) => Some(ServerToProxyMessage::BroadcastChannel(
                hyperion_proxy_proto::BroadcastChannel {
                    channel_id: message.channel_id,
                    exclude: message
                        .exclude
                        .and_then(filter_map_connection_id)
                        .unwrap_or_default(),
                    data: message.data,
                },
            )),
            Self::Unicast(message) => Some(ServerToProxyMessage::Unicast(
                hyperion_proxy_proto::Unicast {
                    stream: filter_map_connection_id(message.stream)?,
                    data: message.data,
                },
            )),
            Self::SetReceiveBroadcasts(message) => {
                Some(ServerToProxyMessage::SetReceiveBroadcasts(
                    hyperion_proxy_proto::SetReceiveBroadcasts {
                        stream: filter_map_connection_id(message.stream)?,
                    },
                ))
            }
            Self::Shutdown(message) => Some(ServerToProxyMessage::Shutdown(
                hyperion_proxy_proto::Shutdown {
                    stream: filter_map_connection_id(message.stream)?,
                },
            )),
        }
    }
}
#[cfg(test)]
mod tests {
    use hyperion_proxy_proto::UpdateChannelPosition;

    use super::*;

    /// Every arm of [`IntermediateServerToProxyMessage::transform_for_proxy`] is a hand-written
    /// copy of the arm above it, so the wire variant it names is exactly the kind of thing that
    /// survives a copy-paste unchanged. That is not a hypothetical: the `Shutdown` arm shipped
    /// building a `SetReceiveBroadcasts`, which turned every failed-validation kick in the
    /// codebase into a broadcast enable for the client that failed. Comparing the two variant
    /// names catches the whole class rather than that one instance.
    const fn intermediate_variant(message: &IntermediateServerToProxyMessage<'_>) -> &'static str {
        match message {
            IntermediateServerToProxyMessage::UpdatePlayerPositions(_) => "UpdatePlayerPositions",
            IntermediateServerToProxyMessage::AddChannel(_) => "AddChannel",
            IntermediateServerToProxyMessage::UpdateChannelPositions(_) => "UpdateChannelPositions",
            IntermediateServerToProxyMessage::RemoveChannel(_) => "RemoveChannel",
            IntermediateServerToProxyMessage::SubscribeChannelPackets(_) => {
                "SubscribeChannelPackets"
            }
            IntermediateServerToProxyMessage::BroadcastGlobal(_) => "BroadcastGlobal",
            IntermediateServerToProxyMessage::BroadcastLocal(_) => "BroadcastLocal",
            IntermediateServerToProxyMessage::BroadcastChannel(_) => "BroadcastChannel",
            IntermediateServerToProxyMessage::Unicast(_) => "Unicast",
            IntermediateServerToProxyMessage::SetReceiveBroadcasts(_) => "SetReceiveBroadcasts",
            IntermediateServerToProxyMessage::Shutdown(_) => "Shutdown",
        }
    }

    const fn proxy_variant(message: &ServerToProxyMessage<'_>) -> &'static str {
        match message {
            ServerToProxyMessage::UpdatePlayerPositions(_) => "UpdatePlayerPositions",
            ServerToProxyMessage::AddChannel(_) => "AddChannel",
            ServerToProxyMessage::UpdateChannelPositions(_) => "UpdateChannelPositions",
            ServerToProxyMessage::RemoveChannel(_) => "RemoveChannel",
            ServerToProxyMessage::SubscribeChannelPackets(_) => "SubscribeChannelPackets",
            ServerToProxyMessage::BroadcastGlobal(_) => "BroadcastGlobal",
            ServerToProxyMessage::BroadcastLocal(_) => "BroadcastLocal",
            ServerToProxyMessage::BroadcastChannel(_) => "BroadcastChannel",
            ServerToProxyMessage::Unicast(_) => "Unicast",
            ServerToProxyMessage::SetReceiveBroadcasts(_) => "SetReceiveBroadcasts",
            ServerToProxyMessage::Shutdown(_) => "Shutdown",
        }
    }

    #[test]
    fn every_variant_transforms_into_its_own_wire_variant() {
        let proxy = ProxyId::new(7);
        let connection = ConnectionId::new(3, proxy);
        let data = &[1u8, 2, 3];
        let updates = &[UpdateChannelPosition {
            channel_id: 1,
            position: ChunkPosition::new(0, 0),
        }];

        // Listing every variant by hand rather than iterating means adding one to the enum
        // without adding it here is a non-exhaustive-match compile error, not a silent gap.
        let messages = [
            IntermediateServerToProxyMessage::UpdatePlayerPositions(UpdatePlayerPositions {
                stream: vec![connection],
                positions: vec![ChunkPosition::new(1, 2)],
            }),
            IntermediateServerToProxyMessage::AddChannel(AddChannel {
                channel_id: 1,
                unsubscribe_packets: data,
            }),
            IntermediateServerToProxyMessage::UpdateChannelPositions(UpdateChannelPositions {
                updates,
            }),
            IntermediateServerToProxyMessage::RemoveChannel(RemoveChannel { channel_id: 1 }),
            IntermediateServerToProxyMessage::SubscribeChannelPackets(SubscribeChannelPackets {
                channel_id: 1,
                exclude: Some(connection),
                data,
            }),
            IntermediateServerToProxyMessage::BroadcastGlobal(BroadcastGlobal {
                exclude: Some(connection),
                data,
            }),
            IntermediateServerToProxyMessage::BroadcastLocal(BroadcastLocal {
                center: ChunkPosition::new(1, 2),
                exclude: Some(connection),
                data,
            }),
            IntermediateServerToProxyMessage::BroadcastChannel(BroadcastChannel {
                channel_id: 1,
                exclude: Some(connection),
                data,
            }),
            IntermediateServerToProxyMessage::Unicast(Unicast {
                stream: connection,
                data,
            }),
            IntermediateServerToProxyMessage::SetReceiveBroadcasts(SetReceiveBroadcasts {
                stream: connection,
            }),
            IntermediateServerToProxyMessage::Shutdown(Shutdown { stream: connection }),
        ];

        for message in &messages {
            let expected = intermediate_variant(message);
            let transformed = message
                .transform_for_proxy(proxy)
                .unwrap_or_else(|| panic!("{expected} must be sent to the proxy that owns it"));
            assert_eq!(
                expected,
                proxy_variant(&transformed),
                "{expected} transformed into a {} on the wire",
                proxy_variant(&transformed)
            );
        }
    }

    /// A shutdown is addressed to one connection, so it must not reach a proxy that connection is
    /// not on. Without this the fix above could have been a `Shutdown` that still fanned out.
    #[test]
    fn shutdown_only_reaches_the_owning_proxy() {
        let message = IntermediateServerToProxyMessage::Shutdown(Shutdown {
            stream: ConnectionId::new(3, ProxyId::new(0)),
        });

        let mine = message
            .transform_for_proxy(ProxyId::new(0))
            .expect("the owning proxy must be told to shut the connection down");
        assert!(matches!(
            mine,
            ServerToProxyMessage::Shutdown(hyperion_proxy_proto::Shutdown { stream: 3 })
        ));

        assert!(
            message.transform_for_proxy(ProxyId::new(1)).is_none(),
            "a shutdown must not reach a proxy the connection is not on"
        );
    }
}
