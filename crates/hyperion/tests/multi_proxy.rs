//! What the game server sends two proxies, as the proxies themselves read it.
//!
//! Every bug this file guards is invisible with one proxy, and each was live on main until the
//! commit that added these: with a single proxy every message reaches the only proxy there is, so
//! nothing that routes to the wrong one can be observed at all. The two halves of the wire types
//! are therefore checked against each other -- the game server's `transform_for_proxy` producing
//! the bytes, and `hyperion-proxy`'s `BufferedEgress` consuming them -- rather than each against
//! its own idea of the format.

use bytes::Bytes;
use hyperion::net::{
    ConnectionId, ProxyId,
    intermediate::{
        BroadcastLocal, IntermediateServerToProxyMessage, UpdatePlayerPosition,
        UpdatePlayerPositions,
    },
};
use hyperion_proxy::{cache::BufferedEgress, data::PlayerHandle, egress::Egress};
use hyperion_proxy_proto::{ArchivedServerToProxyMessage, ChunkPosition, ServerToProxyMessage};
use rustc_hash::FxBuildHasher;

/// A proxy that records what each of its players was sent.
struct Proxy {
    egress: BufferedEgress,
    players: Vec<(u64, kanal::Receiver<Bytes>)>,
}

impl Proxy {
    /// Builds a proxy holding `streams`, each with a channel this test can read.
    fn new(streams: &[u64]) -> Self {
        let registry: &'static _ =
            Box::leak(Box::new(
                papaya::HashMap::<u64, PlayerHandle, FxBuildHasher>::default(),
            ));

        let mut players = Vec::new();
        {
            let pinned = registry.pin();
            for &stream in streams {
                let (tx, rx) = kanal::bounded::<Bytes>(16);
                let handle = PlayerHandle::new(tx.to_async());
                // Local broadcasts are only delivered to players who may receive them.
                handle.enable_receive_broadcasts();
                pinned.insert(stream, handle);
                players.push((stream, rx));
            }
        }

        let (server_tx, _server_rx) = kanal::bounded_async(64);
        Self {
            egress: BufferedEgress::new(Egress::new(registry, server_tx)),
            players,
        }
    }

    /// Hands the proxy a message addressed to it, through rkyv exactly as the socket would.
    fn receive(&mut self, message: &IntermediateServerToProxyMessage<'_>, proxy_id: ProxyId) {
        let Some(message): Option<ServerToProxyMessage<'_>> = message.transform_for_proxy(proxy_id)
        else {
            return;
        };

        let bytes = rkyv::api::high::to_bytes::<rkyv::rancor::Error>(&message).unwrap();
        let archived =
            unsafe { rkyv::access_unchecked::<ArchivedServerToProxyMessage<'_>>(&bytes) };
        self.egress.handle_packet(archived);
    }

    /// The streams that have something waiting for them.
    fn who_was_sent_something(&self) -> Vec<u64> {
        self.players
            .iter()
            .filter(|(_, rx)| !rx.is_empty())
            .map(|(stream, _)| *stream)
            .collect()
    }
}

/// A regional broadcast must reach the players actually near its center.
///
/// Positions used to cross the wire as a `Vec` of streams beside a `Vec` of positions.
/// `transform_for_proxy` filtered the streams to the receiving proxy's players and shipped the
/// positions whole, and the proxy zipped the two: every entry after the first filtered-out one was
/// paired with somebody else's chunk. Here stream 3 is a hundred chunks away and stream 2 is not
/// on this proxy at all, so under the old pairing stream 3 inherited stream 2's position and a
/// broadcast at the origin reached it.
#[test]
fn a_regional_broadcast_reaches_the_players_who_are_actually_there() {
    let near = ProxyId::new(0);
    let far = ProxyId::new(1);

    // Interleaved across the two proxies, which is what made the filter shift the positions.
    let positions =
        IntermediateServerToProxyMessage::UpdatePlayerPositions(UpdatePlayerPositions {
            players: vec![
                UpdatePlayerPosition {
                    stream: ConnectionId::new(1, near),
                    position: ChunkPosition::new(0, 0),
                },
                UpdatePlayerPosition {
                    stream: ConnectionId::new(2, far),
                    position: ChunkPosition::new(0, 0),
                },
                UpdatePlayerPosition {
                    stream: ConnectionId::new(3, near),
                    position: ChunkPosition::new(100, 0),
                },
                UpdatePlayerPosition {
                    stream: ConnectionId::new(4, far),
                    position: ChunkPosition::new(100, 0),
                },
            ],
        });

    let mut proxy = Proxy::new(&[1, 3]);
    proxy.receive(&positions, near);

    let broadcast = IntermediateServerToProxyMessage::BroadcastLocal(BroadcastLocal {
        center: ChunkPosition::new(0, 0),
        exclude: None,
        data: &[1, 2, 3],
    });
    proxy.receive(&broadcast, near);

    assert_eq!(
        proxy.who_was_sent_something(),
        vec![1],
        "only the player at the broadcast's chunk should have received it; stream 3 is a hundred \
         chunks away and is only reachable if it inherited another player's position"
    );
}

/// The other proxy's players are the ones that proxy knows about, at their own positions.
#[test]
fn each_proxy_places_its_own_players() {
    let near = ProxyId::new(0);
    let far = ProxyId::new(1);

    let positions =
        IntermediateServerToProxyMessage::UpdatePlayerPositions(UpdatePlayerPositions {
            players: vec![
                UpdatePlayerPosition {
                    stream: ConnectionId::new(1, near),
                    position: ChunkPosition::new(0, 0),
                },
                UpdatePlayerPosition {
                    stream: ConnectionId::new(2, far),
                    position: ChunkPosition::new(100, 0),
                },
            ],
        });

    let mut proxy = Proxy::new(&[2]);
    proxy.receive(&positions, far);

    let at_origin = IntermediateServerToProxyMessage::BroadcastLocal(BroadcastLocal {
        center: ChunkPosition::new(0, 0),
        exclude: None,
        data: &[1, 2, 3],
    });
    proxy.receive(&at_origin, far);
    assert!(
        proxy.who_was_sent_something().is_empty(),
        "proxy 1's only player is at chunk 100, not at the origin where proxy 0's player is"
    );

    let at_the_player = IntermediateServerToProxyMessage::BroadcastLocal(BroadcastLocal {
        center: ChunkPosition::new(100, 0),
        exclude: None,
        data: &[1, 2, 3],
    });
    proxy.receive(&at_the_player, far);
    assert_eq!(
        proxy.who_was_sent_something(),
        vec![2],
        "a broadcast at a player's own chunk must reach them"
    );
}
