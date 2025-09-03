use std::collections::HashSet;

// BVH removed for stable compatibility
use bytes::Bytes;
use glam::I16Vec2;
use hyperion_proto::ArchivedServerToProxyMessage;
use rustc_hash::FxHashMap;
use tracing::{debug, error};

use crate::egress::Egress;

/// Maximum chunk distance between a packet's center and a player for a local broadcast to be sent to
/// that player
const RADIUS: i16 = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Player {
    stream: u64,
    chunk_position: I16Vec2,
}

// Point trait removed for stable compatibility

// Data trait removed for stable compatibility

pub struct Channel {
    /// List of connection ids that are pending a subscription to this channel
    pending_connections: HashSet<u64>,

    /// List of connection ids that are currently subscribed to this channel
    subscribed_connections: HashSet<u64>,

    unsubscribe_packets: Bytes,
}

#[derive(Default)]
pub struct ChannelManager {
    channels: FxHashMap<u32, Channel>,
}

/// Buffers egress operations for optimized processing.
pub struct BufferedEgress {
    /// Manages channels
    channel_manager: ChannelManager,
    /// Reference to the underlying egress handler.
    egress: Egress,
    /// Simple player list (replaces BVH for stable compatibility)
    players: Vec<Player>,
}

impl BufferedEgress {
    /// Creates a new `BufferedEgress` instance.
    #[must_use]
    pub fn new(egress: Egress) -> Self {
        Self {
            channel_manager: ChannelManager::default(),
            egress,
            players: Vec::new(),
        }
    }

    /// Handles incoming server-to-proxy messages.
    // #[instrument(skip_all)]
    #[expect(clippy::excessive_nesting)]
    pub fn handle_packet(&mut self, message: &ArchivedServerToProxyMessage<'_>) {
        match message {
            ArchivedServerToProxyMessage::UpdatePlayerPositions(packet) => {
                let mut players = Vec::with_capacity(packet.stream.len());

                for (stream, position) in packet.stream.iter().zip(packet.positions.iter()) {
                    let Ok(stream) = rkyv::deserialize::<u64, rkyv::rancor::Error>(stream) else {
                        continue;
                    };
                    let Ok(position) = rkyv::deserialize::<_, rkyv::rancor::Error>(position) else {
                        continue;
                    };
                    let position = I16Vec2::from(position);

                    players.push(Player {
                        stream,
                        chunk_position: position,
                    });
                }

                self.players = players;
            }
            ArchivedServerToProxyMessage::AddChannel(packet) => {
                let unsubscribe_packets = match rkyv::deserialize::<_, rkyv::rancor::Error>(
                    &packet.unsubscribe_packets,
                ) {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "failed to deserialize raw unsubscribe packets from AddChannel: {e}"
                        );
                        return;
                    }
                };
                debug!("adding channel {}", u32::from(packet.channel_id));

                let previous_channel =
                    self.channel_manager
                        .channels
                        .insert(packet.channel_id.into(), Channel {
                            pending_connections: HashSet::new(),
                            subscribed_connections: HashSet::new(),
                            unsubscribe_packets: Bytes::from(unsubscribe_packets),
                        });

                if previous_channel.is_some() {
                    error!(
                        "server sent AddChannel for a channel with the same id as an existing \
                         channel"
                    );
                }
            }
            ArchivedServerToProxyMessage::UpdateChannelPositions(packet) => {
                let mut requested_subscriptions = Vec::new();
                let players = self.egress.player_registry.pin_owned();
                for update in packet.updates.get() {
                    let channel_id = update.channel_id.into();
                    let Some(channel) = self.channel_manager.channels.get_mut(&channel_id) else {
                        error!(
                            "server sent UpdateChannelPositions with an invalid channel id \
                             {channel_id}"
                        );
                        continue;
                    };

                    let Ok(channel_position) =
                        rkyv::deserialize::<_, rkyv::rancor::Error>(&update.position)
                    else {
                        continue;
                    };
                    let channel_position = I16Vec2::from(channel_position);

                    let _min = channel_position - I16Vec2::splat(RADIUS);
                    let _max = channel_position + I16Vec2::splat(RADIUS);

                    // Simple fallback: check all players (less efficient than BVH)
                    let mut should_remain_subscribed = HashSet::new();

                    for player in &self.players {
                        let player_pos = player.chunk_position;
                        let distance = (player_pos - channel_position).abs();
                        if distance.x <= RADIUS && distance.y <= RADIUS {
                            let stream = player.stream;
                            let Some(player) = players.get(&stream) else {
                                error!("bvh contains invalid stream id {stream}");
                                continue;
                            };

                            if !player.can_receive_broadcasts() {
                                continue;
                            }

                            // This stream should be subscribed to this channel...
                            if channel.subscribed_connections.contains(&stream) {
                                // ... and should remain subscribed to this channel
                                should_remain_subscribed.insert(stream);
                            } else {
                                // ... but it is not currently subscribed
                                if channel.pending_connections.is_empty() {
                                    // Request subscribe packets from the server
                                    requested_subscriptions.push(channel_id);
                                }
                                channel.pending_connections.insert(stream);
                            }
                        }
                    }

                    channel.subscribed_connections.retain(|stream| {
                        let should_remain = should_remain_subscribed.contains(stream);

                        if !should_remain {
                            // This stream is currently subscribed. It will be sent unsubscribe
                            // packets and removed from the subscribed connections set
                            self.egress
                                .unicast(*stream, channel.unsubscribe_packets.clone());
                            debug!("unsubscribing player {stream} from channel {channel_id}");
                        }

                        should_remain
                    });
                }

                if !requested_subscriptions.is_empty() {
                    let request = rkyv::to_bytes::<rkyv::rancor::Error>(
                        &hyperion_proto::ProxyToServerMessage::RequestSubscribeChannelPackets(
                            hyperion_proto::RequestSubscribeChannelPackets {
                                channels: &requested_subscriptions,
                            },
                        ),
                    )
                    .unwrap();
                    let server_sender = self.egress.server_sender.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server_sender.send(request).await {
                            error!("failed to send request subscribe channel packets: {e}");
                        }
                    });
                }
            }
            ArchivedServerToProxyMessage::RemoveChannel(packet) => {
                debug!("removing channel {}", u32::from(packet.channel_id));
                let Some(channel) = self
                    .channel_manager
                    .channels
                    .remove(&packet.channel_id.into())
                else {
                    error!("server sent RemoveChannel for a channel that does not exist");
                    return;
                };

                // Unsubscribe all players
                for &stream in &channel.subscribed_connections {
                    self.egress
                        .unicast(stream, channel.unsubscribe_packets.clone());
                }
            }
            ArchivedServerToProxyMessage::SubscribeChannelPackets(packet) => {
                let exclude = u64::from(packet.exclude);
                let channel_id = packet.channel_id.into();
                let data =
                    Bytes::from(rkyv::deserialize::<_, rkyv::rancor::Error>(&packet.data).unwrap());
                let Some(channel) = self.channel_manager.channels.get_mut(&channel_id) else {
                    error!("server sent SubscribeChannelPackets for a channel that does not exist");
                    return;
                };

                for &stream in &channel.pending_connections {
                    if stream == exclude {
                        continue;
                    }

                    debug!("subscribing player {stream} to channel {channel_id}");
                    self.egress.unicast(stream, data.clone());
                }

                channel
                    .subscribed_connections
                    .extend(channel.pending_connections.iter().copied());
                channel.pending_connections.clear();
            }
            ArchivedServerToProxyMessage::BroadcastGlobal(packet) => {
                let data =
                    Bytes::from(rkyv::deserialize::<_, rkyv::rancor::Error>(&packet.data).unwrap());
                let Ok(exclude) = rkyv::deserialize::<u64, rkyv::rancor::Error>(&packet.exclude)
                else {
                    return;
                };

                let players = self.egress.player_registry.pin_owned();

                for (&stream, player) in &players {
                    if !player.can_receive_broadcasts() || stream == exclude {
                        continue;
                    }

                    self.egress.unicast(stream, data.clone());
                }
            }
            ArchivedServerToProxyMessage::BroadcastLocal(packet) => {
                let Ok(center_x) = rkyv::deserialize::<i16, rkyv::rancor::Error>(&packet.center.x)
                else {
                    return;
                };
                let Ok(center_z) = rkyv::deserialize::<i16, rkyv::rancor::Error>(&packet.center.z)
                else {
                    return;
                };
                let Ok(player_id_to_exclude) =
                    rkyv::deserialize::<u64, rkyv::rancor::Error>(&packet.exclude)
                else {
                    return;
                };
                let data =
                    Bytes::from(rkyv::deserialize::<_, rkyv::rancor::Error>(&packet.data).unwrap());

                let position = I16Vec2::new(center_x, center_z);
                let _min = position - I16Vec2::splat(RADIUS);
                let _max = position + I16Vec2::splat(RADIUS);

                // Simple fallback: check all players (less efficient than BVH)
                for player in &self.players {
                    let player_pos = player.chunk_position;
                    let distance = (player_pos - position).abs();
                    if distance.x <= RADIUS && distance.y <= RADIUS {
                        let stream = player.stream;
                        if stream == player_id_to_exclude {
                            continue;
                        }

                        self.egress.unicast(stream, data.clone());
                    }
                }
            }
            ArchivedServerToProxyMessage::BroadcastChannel(packet) => {
                let exclude = u64::from(packet.exclude);
                let data =
                    Bytes::from(rkyv::deserialize::<_, rkyv::rancor::Error>(&packet.data).unwrap());

                let Some(channel) = self.channel_manager.channels.get(&packet.channel_id.into())
                else {
                    error!("server sent BroadcastChannel for a channel that does not exist");
                    return;
                };

                for &stream in &channel.subscribed_connections {
                    if stream == exclude {
                        continue;
                    }

                    self.egress.unicast(stream, data.clone());
                }
            }
            ArchivedServerToProxyMessage::Unicast(unicast) => {
                let data = rkyv::deserialize::<_, rkyv::rancor::Error>(&unicast.data).unwrap();
                self.egress
                    .unicast(unicast.stream.into(), Bytes::from(data));
            }
            ArchivedServerToProxyMessage::SetReceiveBroadcasts(pkt) => {
                self.egress.handle_set_receive_broadcasts(pkt);
            }
            ArchivedServerToProxyMessage::Shutdown(pkt) => {
                self.egress.handle_shutdown(pkt);
            }
        }
    }
}
