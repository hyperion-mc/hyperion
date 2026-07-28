use hyperion_proxy_proto::{ChunkPosition, ServerToProxyMessage, UpdateChannelPosition};

use crate::net::{ConnectionId, ProxyId};

#[derive(Clone, Copy, PartialEq)]
pub struct UpdatePlayerPosition {
    pub stream: ConnectionId,
    pub position: ChunkPosition,
}

/// One entry per player. See [`hyperion_proxy_proto::UpdatePlayerPositions`] for why this is not
/// two parallel `Vec`s: `transform_for_proxy` filters this list, and a filter over one of two
/// parallel lists silently desynchronises them.
#[derive(Clone, PartialEq)]
pub struct UpdatePlayerPositions {
    pub players: Vec<UpdatePlayerPosition>,
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

    /// The proxy that asked for these packets, and the only one they are sent to.
    pub receiver: ProxyId,

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
                        players: message
                            .players
                            .iter()
                            .filter_map(|player| {
                                Some(hyperion_proxy_proto::UpdatePlayerPosition {
                                    stream: filter_map_connection_id(player.stream)?,
                                    position: player.position,
                                })
                            })
                            .collect::<Vec<_>>(),
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
                // Only the proxy that asked has a player waiting on the answer. Delivering it to
                // the others subscribes their players to a channel nobody asked them about.
                if message.receiver != proxy_id {
                    return None;
                }

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
                players: vec![UpdatePlayerPosition {
                    stream: connection,
                    position: ChunkPosition::new(1, 2),
                }],
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
                receiver: proxy,
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

            // `affected_by_proxy` is a hand-written list of which arms below read `proxy_id`, and
            // nothing else checks the two agree. Getting it wrong is not loud: the `false` branch
            // of `IoBuf::add_proxy_message` encodes once against `ProxyId::new(0)`, which is the
            // real id of the first proxy to connect, so a variant that does depend on the proxy
            // but is listed as not would quietly route everything as proxy 0.
            if !message.affected_by_proxy() {
                let elsewhere = message
                    .transform_for_proxy(ProxyId::new(99))
                    .map(|sent| proxy_variant(&sent));
                assert_eq!(
                    Some(proxy_variant(&transformed)),
                    elsewhere,
                    "{expected} says it does not depend on the proxy, but transforms differently \
                     for two of them"
                );
            }
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

    /// Positions used to travel as a `Vec` of streams beside a `Vec` of positions, and
    /// `transform_for_proxy` filtered the streams but shipped the positions whole. The proxy
    /// zipped them, so with two proxies every player landed at somebody else's chunk in the
    /// regional-broadcast BVH.
    ///
    /// Interleaving the two proxies' players is what makes this bite: a filter that drops entry 0
    /// shifts every later position by one.
    #[test]
    fn each_proxy_sees_its_own_players_at_their_own_positions() {
        let zero = ProxyId::new(0);
        let one = ProxyId::new(1);

        // Interleaved, and each player's chunk x is their stream id, so a pairing error is
        // impossible to read as anything else.
        let players = vec![
            UpdatePlayerPosition {
                stream: ConnectionId::new(10, zero),
                position: ChunkPosition::new(10, 0),
            },
            UpdatePlayerPosition {
                stream: ConnectionId::new(20, one),
                position: ChunkPosition::new(20, 0),
            },
            UpdatePlayerPosition {
                stream: ConnectionId::new(30, zero),
                position: ChunkPosition::new(30, 0),
            },
            UpdatePlayerPosition {
                stream: ConnectionId::new(40, one),
                position: ChunkPosition::new(40, 0),
            },
        ];
        let message =
            IntermediateServerToProxyMessage::UpdatePlayerPositions(UpdatePlayerPositions {
                players,
            });

        for (proxy, expected) in [(zero, [10u64, 30]), (one, [20, 40])] {
            let Some(ServerToProxyMessage::UpdatePlayerPositions(sent)) =
                message.transform_for_proxy(proxy)
            else {
                panic!("every proxy is told where its own players are");
            };

            let streams = sent.players.iter().map(|p| p.stream).collect::<Vec<_>>();
            assert_eq!(
                streams,
                expected.to_vec(),
                "proxy {} was sent the wrong players",
                proxy.inner()
            );

            for player in &sent.players {
                assert_eq!(
                    i64::from(player.position.x),
                    i64::try_from(player.stream).unwrap(),
                    "player {} was sent to another player's chunk",
                    player.stream
                );
            }
        }
    }

    /// A subscribe answers one proxy's request. `SubscribeChannelPackets` was listed in
    /// `affected_by_proxy` but transformed unconditionally, so the answer also went to proxies
    /// that never asked and subscribed their players to a channel nobody had requested.
    #[test]
    fn a_subscribe_reaches_only_the_proxy_that_asked() {
        let asker = ProxyId::new(0);
        let bystander = ProxyId::new(1);

        let message =
            IntermediateServerToProxyMessage::SubscribeChannelPackets(SubscribeChannelPackets {
                channel_id: 7,
                exclude: None,
                receiver: asker,
                data: &[1, 2, 3],
            });

        assert!(
            matches!(
                message.transform_for_proxy(asker),
                Some(ServerToProxyMessage::SubscribeChannelPackets(sent))
                    if sent.channel_id == 7
            ),
            "the proxy that asked must get its answer"
        );
        assert!(
            message.transform_for_proxy(bystander).is_none(),
            "a proxy that never asked must not be told to subscribe anyone"
        );
    }
}
