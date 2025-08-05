use std::collections::HashSet;

use bvh::{Aabb, Bvh, Data, Point};
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

impl Point for Player {
    fn point(&self) -> I16Vec2 {
        self.chunk_position
    }
}

impl Data for Player {
    type Unit = u64;

    fn data<'a: 'c, 'b: 'c, 'c>(&'a self, (): Self::Context<'b>) -> &'c [Self::Unit] {
        std::slice::from_ref(&self.stream)
    }
}

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
    player_bvh: Bvh<Vec<u64>>,
}

impl BufferedEgress {
    /// Creates a new `BufferedEgress` instance.
    #[must_use]
    pub fn new(egress: Egress) -> Self {
        Self {
            channel_manager: ChannelManager::default(),
            egress,
            player_bvh: Bvh::default(),
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
                    let Ok(stream) = rkyv::deserialize::<u64, !>(stream);
                    let Ok(position) = rkyv::deserialize::<_, !>(position);
                    let position = I16Vec2::from(position);

                    players.push(Player {
                        stream,
                        chunk_position: position,
                    });
                }

                self.player_bvh = Bvh::build(&mut players, ());
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

                    let Ok(channel_position) = rkyv::deserialize::<_, !>(&update.position);
                    let channel_position = I16Vec2::from(channel_position);

                    let min = channel_position - I16Vec2::splat(RADIUS);
                    let max = channel_position + I16Vec2::splat(RADIUS);

                    let aabb = Aabb::new(min, max);

                    let slices = self.player_bvh.get_in(aabb);

                    let mut should_remain_subscribed = HashSet::new();

                    for slice in slices {
                        let (_, streams) = self.player_bvh.inner();

                        let start = slice.start as usize;
                        let end = slice.end as usize;

                        let streams = &streams[start..end];
                        for &stream in streams {
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
                let Ok(exclude) = rkyv::deserialize::<u64, !>(&packet.exclude);

                let players = self.egress.player_registry.pin_owned();

                for (&stream, player) in &players {
                    if !player.can_receive_broadcasts() || stream == exclude {
                        continue;
                    }

                    self.egress.unicast(stream, data.clone());
                }
            }
            ArchivedServerToProxyMessage::BroadcastLocal(packet) => {
                let Ok(center_x) = rkyv::deserialize::<i16, !>(&packet.center.x);
                let Ok(center_z) = rkyv::deserialize::<i16, !>(&packet.center.z);
                let Ok(player_id_to_exclude) = rkyv::deserialize::<u64, !>(&packet.exclude);
                let data =
                    Bytes::from(rkyv::deserialize::<_, rkyv::rancor::Error>(&packet.data).unwrap());

                let position = I16Vec2::new(center_x, center_z);
                let min = position - I16Vec2::splat(RADIUS);
                let max = position + I16Vec2::splat(RADIUS);

                let aabb = Aabb::new(min, max);

                let slices = self.player_bvh.get_in(aabb);

                for slice in slices {
                    let (_, streams) = self.player_bvh.inner();

                    let start = slice.start as usize;
                    let end = slice.end as usize;

                    let streams = &streams[start..end];
                    for &stream in streams {
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
