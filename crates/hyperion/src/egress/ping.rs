//! Per-player round trip time, and the ping bars the tab list draws from it.
//!
//! # There was no measurement to be stale
//!
//! `roster::entry_of` sent `ping: 0` and `list.rs` forwarded it once, at join,
//! and that was the whole of it. The reason was not a forgotten update: it was
//! that **nothing on either side of the connection ever asked**. The game
//! server routed a serverbound `keep_alive` to `Route::Ignore` and never sent a
//! clientbound one, so the client -- which only ever answers a keep-alive and
//! never starts one -- had nothing to answer. `0` was not a reading taken once
//! and left; it was the absence of a reading, drawn as a full five bars.
//!
//! # The proxy does not answer keep-alives, so this measures the player
//!
//! This is the one place the design could quietly lie. hyperion puts a proxy
//! between the client and the game server, and a proxy that answered
//! keep-alives itself would leave the game server timing the *proxy*, which
//! would look like a plausible ping and be a measurement of the wrong thing.
//!
//! It does not. `crates/hyperion-proxy` contains no keep-alive handling at all
//! -- the player-to-server direction in `player.rs` reads bytes off the socket
//! and forwards the frames without parsing an id -- but that is an argument,
//! not a measurement. `tools/tab-list-check.py` is the measurement: a real
//! client answers keep-alives, watches a latency arrive, then **stops
//! answering** while staying otherwise busy. The reading falls back to
//! "unknown", and comes back when it answers again. Nothing between the client
//! and the game server can produce that, so the thing being timed is the
//! player.
//!
//! # What is in the number: one tick of it is this server
//!
//! Send to encode here, out through the proxy, to the client, back through the
//! proxy, and in again as far as the tick that decodes it. Two things ride
//! along with the network time and both are named rather than subtracted out:
//!
//! - **the proxy hops**, which are part of what a player waits for and belong
//!   in a number labelled "ping",
//! - **one whole tick of inbound scheduling**, because `ingress::decode`'s
//!   `recv_data` drains queued frames once per tick and the probe goes out at
//!   the end of one, so an answer cannot be seen before the next.
//!
//! The second is not a worst case, it is the floor, and it is big. Measured in
//! the gate, on loopback, where the true round trip is under a millisecond:
//! **58 ms and 61 ms**, against a tick of 59 ms on a loop that was managing
//! 17 tps. Essentially all of the reading was this server waiting for its own
//! next tick.
//!
//! So read the number as *the player's round trip plus about a tick*, and note
//! what that costs at the bar thresholds below: with a ~50-60 ms offset, a
//! player whose real ping is 100 ms is drawn at four bars rather than five.
//! The bar is never wrong by more than one step, and it is biased one way.
//!
//! Removing it means timestamping a frame when it arrives rather than when it
//! is decoded, which is a change to the packet channel every connection shares
//! and is deliberately not made here. Having the *proxy* stamp arrival is the
//! other option and is worse: it needs a clock shared by two hosts to mean
//! anything.
//!
//! # One probe at a time
//!
//! A new keep-alive goes out only once the last one is answered or has timed
//! out, so an id can never be ambiguous and a slow client is probed less
//! rather than being handed a queue it cannot drain. A probe unanswered for
//! [`Global::keep_alive_timeout`](crate::Global::keep_alive_timeout) drops the
//! reading back to "no reading" and starts a new one. hyperion still does not
//! disconnect anyone over it -- `simulation::handlers` says the same -- so the
//! effect is confined to the readout.

use std::time::{Duration, Instant};

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::clientbound::KeepAlive,
};
use tracing::error;

use crate::{
    egress::player_join::{PlayerInfoActions, PlayerList, PlayerListEntry},
    net::{Compose, ConnectionId, protocol::send},
    simulation::{PacketState, Uuid},
};

/// How long to wait, after a probe is answered, before sending the next.
///
/// Two seconds costs one ten-byte packet per player per two seconds -- 5 KB/s
/// across ten thousand players, against the per-player-per-tick position
/// updates the same link already carries -- and bounds how stale a bar can be
/// at two seconds plus one round trip. Probing faster would buy resolution the
/// five-bar display cannot show.
const PERIOD: Duration = Duration::from_secs(2);

/// The latency that means "no reading", which is the client's own
/// `PING_UNKNOWN_SPRITE` case rather than a number invented to stand in for
/// one.
pub const UNKNOWN: i32 = -1;

/// The ping bar the vanilla client draws for a latency in milliseconds.
///
/// Read off `PlayerTabOverlay.extractPingIcon` in the 26.2 client jar (sha1
/// `2dc72797acbc1b63fc16a11c4ac393605f453754`, which is the jar
/// `nix/minecraft-version.json` already pins):
///
/// ```java
/// Identifier sprite = info.getLatency() < 0 ? PING_UNKNOWN_SPRITE
///     : (info.getLatency() < 150 ? PING_5_SPRITE
///     : (info.getLatency() < 300 ? PING_4_SPRITE
///     : (info.getLatency() < 600 ? PING_3_SPRITE
///     : (info.getLatency() < 1000 ? PING_2_SPRITE : PING_1_SPRITE))));
/// ```
///
/// Six sprites, five of them bars. Vanilla draws the icon and never the
/// number, so this enum is the whole of what a player can see, which is why it
/// and not the millisecond is what decides whether an update is worth sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bars {
    /// No reading yet, or the last probe timed out.
    Unknown,
    /// A second or worse.
    One,
    /// 600..1000 ms.
    Two,
    /// 300..600 ms.
    Three,
    /// 150..300 ms.
    Four,
    /// Under 150 ms.
    Five,
}

/// Which bar `latency` milliseconds draws.
#[must_use]
pub const fn bars(latency: i32) -> Bars {
    if latency < 0 {
        Bars::Unknown
    } else if latency < 150 {
        Bars::Five
    } else if latency < 300 {
        Bars::Four
    } else if latency < 600 {
        Bars::Three
    } else if latency < 1000 {
        Bars::Two
    } else {
        Bars::One
    }
}

/// A probe waiting for its answer.
#[derive(Debug, Clone, Copy)]
struct Probe {
    /// The value the client echoes back.
    id: i64,
    /// When it went out.
    sent: Instant,
}

/// One player's round trip time.
#[derive(Component, Debug, Default)]
pub struct Ping {
    /// The probe waiting to be answered, if any.
    pending: Option<Probe>,
    /// When the last probe went out, answered or not.
    last_probe: Option<Instant>,
    /// Strictly increasing per connection, so an answer to a probe that
    /// already timed out cannot be mistaken for the current one.
    next_id: i64,
    /// The last measured round trip.
    ///
    /// `None` before the first answer and again after one times out, which is
    /// the difference between "not measured" and "measured as fast".
    pub rtt: Option<Duration>,
    /// The latency this player's tab list entry was last published with.
    published: Option<i32>,
}

impl Ping {
    /// The latency to put on the wire: whole milliseconds, or [`UNKNOWN`].
    ///
    /// The real measurement and not the bucket. Only the *resend* is quantised
    /// to bars, so anything that reads the number -- a client that draws it,
    /// a mod, a capture -- gets a true reading that is merely updated at bar
    /// granularity, rather than a rounded one.
    #[must_use]
    pub fn latency(&self) -> i32 {
        let Some(rtt) = self.rtt else {
            return UNKNOWN;
        };
        i32::try_from(rtt.as_millis()).unwrap_or(i32::MAX)
    }

    /// The id of the probe to send now, if one is due.
    fn probe(&mut self, now: Instant, period: Duration, timeout: Duration) -> Option<i64> {
        if let Some(pending) = self.pending {
            if now.duration_since(pending.sent) < timeout {
                return None;
            }
            // Nothing came back in time, so there is no reading any more.
            // Keeping the old one would draw a live bar for a client that has
            // gone quiet, which is the one thing a ping display must not do.
            self.rtt = None;
            self.pending = None;
        }

        if let Some(last) = self.last_probe
            && now.duration_since(last) < period
        {
            return None;
        }

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending = Some(Probe { id, sent: now });
        self.last_probe = Some(now);
        Some(id)
    }

    /// Fold an answer in, and say whether it was the one being waited on.
    ///
    /// A client that echoes something else, or answers twice, moves nothing:
    /// a reading is only taken for the probe currently outstanding.
    fn answer(&mut self, id: i64, now: Instant) -> bool {
        let Some(pending) = self.pending else {
            return false;
        };
        if pending.id != id {
            return false;
        }
        self.pending = None;
        self.rtt = Some(now.saturating_duration_since(pending.sent));
        true
    }

    /// Whether the bar this player draws differs from the one every client was
    /// last told to draw.
    fn moved(&self) -> bool {
        self.published
            .is_none_or(|published| bars(published) != bars(self.latency()))
    }
}

/// Fold a client's keep-alive answer into its [`Ping`].
///
/// Called from `simulation::handlers`, which owns the serverbound routing
/// table; the measurement lives here with the probe that started it.
pub fn absorb_answer(entity: EntityView<'_>, id: i64) {
    entity.try_get::<&mut Ping>(|ping| {
        ping.answer(id, Instant::now());
    });
}

/// Registration module for the ping readout: the [`Ping`] component.
///
/// Registration only, per the flecs convention in the root `CLAUDE.md`, and
/// deliberately only the type. The other half of the wiring -- that every
/// `Player` carries a `Ping`, so nothing on the join path has to remember to
/// add one -- is a statement about `Player`, so it is declared by the module
/// that owns `Player`, alongside the identical statements about `CursorItem`
/// and `InventoryState`. `SimComponentsModule` imports this for the component
/// to point at.
#[derive(Component)]
pub struct PingComponentsModule;

impl Module for PingComponentsModule {
    fn module(world: &World) {
        world.component::<Ping>();
    }
}

/// Behavior module for the ping readout: the probe that measures and the
/// change-only update that publishes.
#[derive(Component)]
pub struct PingModule;

impl Module for PingModule {
    fn module(world: &World) {
        world.import::<PingComponentsModule>();

        // PreStore, so an answer decoded this tick in OnUpdate is folded in
        // before this decides whether to probe again or what to publish.
        system!("probe_ping", world, &Compose, &ConnectionId, &mut Ping)
            .with_enum(PacketState::Play)
            .kind(id::<flecs::pipeline::PreStore>())
            .each(|(compose, connection_id, ping)| {
                let timeout = compose.global().keep_alive_timeout;
                let Some(id) = ping.probe(Instant::now(), PERIOD, timeout) else {
                    return;
                };
                let packet = KeepAlive(id);
                if let Err(error) = send(
                    compose,
                    *connection_id,
                    PacketId::KeepAlive.to_raw(),
                    &packet,
                ) {
                    error!("failed to send a keep-alive probe: {error}");
                }
            });

        let players = world
            .query::<(&Uuid, &mut Ping)>()
            .with_enum(PacketState::Play)
            .build();

        // One packet for everyone whose bar moved, not one packet per player
        // and not the whole roster: `PlayerInfoUpdate` carries a list, and
        // `UPDATE_LATENCY` alone writes a uuid and an int per entry.
        world
            .system_named::<&Compose>("publish_ping")
            .kind(id::<flecs::pipeline::PreStore>())
            .each(move |compose| {
                let mut entries = Vec::new();
                players.each(|(uuid, ping)| {
                    if ping.moved() {
                        entries.push(PlayerListEntry {
                            uuid: uuid.0,
                            ping: ping.latency(),
                            ..PlayerListEntry::default()
                        });
                    }
                });
                if entries.is_empty() {
                    return;
                }

                let update = PlayerList {
                    actions: PlayerInfoActions::UPDATE_LATENCY,
                    entries,
                };
                if let Err(error) = compose.broadcast(&update).send() {
                    error!("failed to publish ping updates: {error}");
                    // `published` is left alone, so the same set goes out
                    // again next tick rather than being recorded as delivered.
                    return;
                }

                players.each(|(_, ping)| {
                    if ping.moved() {
                        let latency = ping.latency();
                        ping.published = Some(latency);
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every threshold in `extractPingIcon`, on both sides. An off-by-one here
    /// draws the wrong icon and nothing else notices.
    #[test]
    fn the_buckets_are_the_clients_own_thresholds() {
        assert_eq!(bars(-1), Bars::Unknown);
        assert_eq!(bars(UNKNOWN), Bars::Unknown);
        assert_eq!(bars(0), Bars::Five);
        assert_eq!(bars(149), Bars::Five);
        assert_eq!(bars(150), Bars::Four);
        assert_eq!(bars(299), Bars::Four);
        assert_eq!(bars(300), Bars::Three);
        assert_eq!(bars(599), Bars::Three);
        assert_eq!(bars(600), Bars::Two);
        assert_eq!(bars(999), Bars::Two);
        assert_eq!(bars(1000), Bars::One);
        assert_eq!(bars(i32::MAX), Bars::One);
    }

    /// A round trip is the time between the probe going out and its own answer
    /// coming back.
    #[test]
    fn an_answered_probe_is_the_time_it_took() {
        let mut ping = Ping::default();
        let start = Instant::now();

        assert_eq!(ping.latency(), UNKNOWN, "nothing has been measured yet");
        let id = ping.probe(start, PERIOD, Duration::from_secs(20)).unwrap();

        assert!(ping.answer(id, start + Duration::from_millis(42)));
        assert_eq!(ping.rtt, Some(Duration::from_millis(42)));
        assert_eq!(ping.latency(), 42);
        assert_eq!(bars(ping.latency()), Bars::Five);
    }

    /// Only one probe is outstanding, so a second is not sent until the first
    /// is answered and the period has passed.
    #[test]
    fn a_probe_waits_for_its_answer_and_then_for_the_period() {
        let mut ping = Ping::default();
        let start = Instant::now();
        let timeout = Duration::from_secs(20);

        let first = ping.probe(start, PERIOD, timeout).unwrap();
        // Unanswered: nothing else goes out however long it has been, short of
        // the timeout.
        assert_eq!(ping.probe(start + PERIOD * 2, PERIOD, timeout), None);

        assert!(ping.answer(first, start + Duration::from_millis(10)));
        // Answered, but the period has not passed.
        assert_eq!(
            ping.probe(start + Duration::from_millis(20), PERIOD, timeout),
            None
        );

        let second = ping.probe(start + PERIOD, PERIOD, timeout).unwrap();
        assert_ne!(
            first, second,
            "ids must not repeat while a stale one is live"
        );
    }

    /// An echo of something else moves nothing. A client answering a probe
    /// that already timed out must not be credited to the one now in flight.
    #[test]
    fn an_answer_to_the_wrong_probe_is_ignored() {
        let mut ping = Ping::default();
        let start = Instant::now();
        let timeout = Duration::from_secs(20);

        let first = ping.probe(start, PERIOD, timeout).unwrap();
        assert!(!ping.answer(first.wrapping_add(1), start + Duration::from_millis(5)));
        assert_eq!(ping.rtt, None);

        // The real one still lands.
        assert!(ping.answer(first, start + Duration::from_millis(5)));
        assert_eq!(ping.latency(), 5);

        // And a repeat of it does not, because nothing is outstanding.
        assert!(!ping.answer(first, start + Duration::from_secs(9)));
        assert_eq!(ping.latency(), 5);
    }

    /// A client that stops answering loses its reading rather than keeping a
    /// stale live bar, and is probed again.
    #[test]
    fn a_timed_out_probe_drops_the_reading() {
        let mut ping = Ping::default();
        let start = Instant::now();
        let timeout = Duration::from_secs(20);

        let first = ping.probe(start, PERIOD, timeout).unwrap();
        assert!(ping.answer(first, start + Duration::from_millis(30)));
        assert_eq!(bars(ping.latency()), Bars::Five);

        let second = ping.probe(start + PERIOD, PERIOD, timeout).unwrap();
        // Still inside the timeout: the last good reading stands.
        assert_eq!(ping.probe(start + PERIOD * 2, PERIOD, timeout), None);
        assert_eq!(bars(ping.latency()), Bars::Five);

        // Past it: no reading, and a fresh probe.
        let third = ping
            .probe(start + PERIOD + timeout, PERIOD, timeout)
            .unwrap();
        assert_eq!(ping.rtt, None);
        assert_eq!(ping.latency(), UNKNOWN);
        assert_eq!(bars(ping.latency()), Bars::Unknown);
        assert_ne!(second, third);
    }

    /// An update is sent when the bar moves and not when the millisecond does,
    /// which is the whole reason the roster is not re-sent every tick.
    #[test]
    fn a_millisecond_that_does_not_move_a_bar_publishes_nothing() {
        let mut ping = Ping::default();
        let start = Instant::now();
        let timeout = Duration::from_secs(20);

        // Nothing published yet, so the first reading always goes out --
        // including the "unknown" a player has before their first answer.
        assert!(ping.moved());
        ping.published = Some(ping.latency());
        assert!(!ping.moved());

        // A real reading in a different bucket moves the bar.
        let id = ping.probe(start, PERIOD, timeout).unwrap();
        assert!(ping.answer(id, start + Duration::from_millis(40)));
        assert!(ping.moved());
        ping.published = Some(ping.latency());

        // 40 ms to 120 ms is a big change in the number and no change at all
        // in what the player sees, so it sends nothing.
        let id = ping.probe(start + PERIOD, PERIOD, timeout).unwrap();
        assert!(ping.answer(id, start + PERIOD + Duration::from_millis(120)));
        assert_eq!(ping.latency(), 120);
        assert!(!ping.moved());

        // Crossing 150 ms does.
        let id = ping.probe(start + PERIOD * 2, PERIOD, timeout).unwrap();
        assert!(ping.answer(id, start + PERIOD * 2 + Duration::from_millis(150)));
        assert_eq!(bars(ping.latency()), Bars::Four);
        assert!(ping.moved());
    }
}
