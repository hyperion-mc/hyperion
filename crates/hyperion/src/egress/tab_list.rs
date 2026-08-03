//! The tab list header and footer, and the server's own tick rate in it.
//!
//! Server telemetry is not a game concept, which is why the tick rate lives
//! here and not in an event crate -- the same reasoning as
//! [`crate::egress::server_load`], and the same honesty rule adapted to a
//! surface that has no bar to fill:
//!
//! > **both numbers are printed, the first is what the loop did and the second
//! > is what it was paced to.**
//!
//! ```text
//! TPS 19.8 / 20.0
//! 3 players online
//! ```
//!
//! A server keeping up prints them equal. That is the only way `20.0` can
//! appear, so a vanity constant cannot masquerade as a measurement: it would
//! have to survive [`Tps::absorb`], which counts real ticks.
//!
//! # Why a count of ticks and not flecs's frame time
//!
//! `world.info().frame_time_total` is the time spent *inside* frames, so it
//! excludes the sleep flecs does to hold 20 Hz. A rate derived from it answers
//! "how fast could this server tick" and reads near-infinite on an idle one,
//! which is a different question wearing the same units. Ticks per wall-clock
//! second is the number an operator means, and the only way to get it is to
//! count ticks against a clock that does not stop.
//!
//! One sample cannot carry a rate, so the first [`WINDOW`] reports
//! `TPS sampling` rather than a guess -- the same refusal
//! [`server_load`](crate::egress::server_load) makes for its first CPU window.
//!
//! # Who writes what
//!
//! The header is left for the event crate; hyperion writes the footer. The
//! split is arbitrary but it has to be *somewhere*, because both halves ride
//! in one packet and two writers with no rule fight every tick. That is not
//! hypothetical: bedwars used to broadcast a whole `TabList` unconditionally,
//! every tick, to every player.
//!
//! Nothing is sent unless the rendered text changed, which is what makes the
//! per-tick broadcast go away: at a steady 20.00 tps the label is stable and
//! this module sends nothing at all. A joining client is unicast the current
//! text instead, because a change-only broadcast is invisible to anyone who
//! was not connected when it happened.

use std::{
    collections::VecDeque,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId,
    packets::play::clientbound::TabList as TabListPacket, text::NamedColor,
};
use tracing::error;

// Re-exported so an event crate writing the header names the text type
// without reaching into the boss bar module for it.
pub use crate::egress::boss_bar::Text;
use crate::{
    TICKS_PER_SECOND,
    net::{Channel, Compose, ConnectionId, protocol::Clientbound},
};

/// How long a tick-rate window is.
///
/// Five seconds and not one: a one second window at 20 tps counts twenty
/// ticks, so a single scheduler hiccup reads as 19.0 and the footer flickers
/// at a number nobody can act on. A hundred ticks resolves to 0.2 tps, which
/// is finer than the one decimal the label prints.
const WINDOW: Duration = Duration::from_secs(5);

/// The rolling window the tick rate is measured over.
#[derive(Component, Debug, Default)]
pub struct Tps {
    /// When each tick inside the open window ran, oldest first.
    ticks: VecDeque<Instant>,
    /// The very first tick, which is the only thing that can say whether this
    /// server has been up for a whole window yet.
    ///
    /// Not derivable from `ticks`, and assuming otherwise is the bug this
    /// field exists to fix: entries older than [`WINDOW`] are dropped, so the
    /// oldest one retained is always *inside* the window and the span between
    /// it and now is always a little short of a whole one. Gating on that span
    /// therefore never opens -- except on perfectly regular ticks, where an
    /// entry lands exactly on the boundary and is kept. That is precisely the
    /// fixture the unit tests used, so they passed while a real server showed
    /// `TPS sampling` forever.
    first: Option<Instant>,
    /// Ticks per second over the last full window.
    ///
    /// `None` until [`WINDOW`] has elapsed. A partial window divided by the
    /// whole window reads low, and a partial window divided by itself is one
    /// sample pretending to be a rate.
    pub rate: Option<f32>,
}

impl Tps {
    /// Fold one tick in.
    fn absorb(&mut self, now: Instant) {
        self.ticks.push_back(now);
        let first = *self.first.get_or_insert(now);

        while let Some(&oldest) = self.ticks.front() {
            if now.duration_since(oldest) > WINDOW {
                self.ticks.pop_front();
            } else {
                break;
            }
        }

        // Whether a whole window has passed is a question about the clock, not
        // about what survived the pop above. See [`Self::first`].
        if now.duration_since(first) < WINDOW {
            return;
        }

        let Some(&oldest) = self.ticks.front() else {
            return;
        };
        let span = now.duration_since(oldest);
        if span.is_zero() {
            return;
        }

        // `n` timestamps span `n - 1` intervals, and it is the intervals that
        // have a rate. Counting the timestamps instead reports 20.2 tps on a
        // server holding exactly 20, because both endpoints of a closed window
        // are inside it.
        //
        // The rate is taken against the span of the entries actually retained
        // rather than against `WINDOW`, so dropping the oldest tick does not
        // read as a slower server. The subtraction cannot wrap: a non-zero
        // span needs two distinct entries.
        let intervals = self.ticks.len().saturating_sub(1);
        self.rate = Some(intervals as f32 / span.as_secs_f32());
    }
}

/// The two halves of the tab list, as the text they will be sent as.
///
/// [`Text`] and never a `String` for the reason
/// [`boss_bar`](crate::egress::boss_bar) gives: a component cannot smuggle a
/// colour in as `§` markup.
#[derive(Component, Debug)]
pub struct TabList {
    /// Drawn above the player list. Left for the event crate.
    pub header: Text,
    /// Drawn below it. Written by [`TabListModule`].
    pub footer: Text,
    /// What every client currently has, so a tick that changes nothing sends
    /// nothing.
    sent: Option<(Text, Text)>,
}

impl Default for TabList {
    fn default() -> Self {
        Self {
            header: Text::text(""),
            footer: Text::text(""),
            sent: None,
        }
    }
}

impl TabList {
    /// Whether the text differs from what the clients were last told.
    fn changed(&self) -> bool {
        !self
            .sent
            .as_ref()
            .is_some_and(|(header, footer)| *header == self.header && *footer == self.footer)
    }
}

/// The footer for a server that measured `rate` ticks per second while paced
/// to `target`, with `players` connected.
///
/// Both numbers, always, and one colour regardless of either: a colour change
/// is an alarm, and `server_load`'s note on why its bars have no thresholds
/// applies here unchanged.
#[must_use]
pub fn footer_readout(rate: Option<f32>, target: f32, players: usize) -> Text {
    let plural = if players == 1 { "player" } else { "players" };
    let tail = Text::text(format!("\n{players} {plural} online")).color(NamedColor::Gray);

    let Some(rate) = rate else {
        return Text::text("TPS sampling")
            .color(NamedColor::Gray)
            .append(tail);
    };
    Text::text(format!("TPS {rate:.1} / {target:.1}"))
        .color(NamedColor::Aqua)
        .append(tail)
}

/// Send the current header and footer to one client.
fn unicast(compose: &Compose, list: &TabList, connection_id: ConnectionId) -> anyhow::Result<()> {
    let packet = TabListPacket {
        header: list.header.to_tag(),
        footer: list.footer.to_tag(),
    };
    compose.unicast(
        Clientbound::new(PacketId::TabList.to_raw(), &packet),
        connection_id,
    )
}

/// Registration module for the tab list: the [`TabList`] text and the [`Tps`]
/// window, both singletons.
///
/// Registration only, per the flecs convention in the root `CLAUDE.md`. An
/// event crate that wants to write the header imports this and nothing else.
#[derive(Component)]
pub struct TabListComponentsModule;

impl Module for TabListComponentsModule {
    fn module(world: &World) {
        // Registered with the trait before the value is set. A bare `set`
        // stores the value without registering the type, which is the
        // dev-only ECS_INVALID_OPERATION abort of ENG-11000.
        world.component::<Tps>().add_trait::<flecs::Singleton>();
        world.set(Tps::default());

        world.component::<TabList>().add_trait::<flecs::Singleton>();
        world.set(TabList::default());
    }
}

/// Behavior module for the tab list: the tick sampler, the change-only
/// broadcast, and the unicast that catches a joining client up.
#[derive(Component)]
pub struct TabListModule;

impl Module for TabListModule {
    fn module(world: &World) {
        world.import::<TabListComponentsModule>();

        // PreStore, so a reading taken this tick is on `TabList` before
        // `tab_list_sync` runs in OnStore and reaches the wire the same tick
        // rather than the next one. Same placement, and the same reason, as
        // `server_load_sample`.
        world
            .system_named::<(&Compose, &mut Tps, &mut TabList)>("tab_list_sample")
            .kind(id::<flecs::pipeline::PreStore>())
            .each(|(compose, tps, list)| {
                tps.absorb(Instant::now());
                let players = compose.global().player_count.load(Ordering::Relaxed);
                let footer = footer_readout(tps.rate, TICKS_PER_SECOND, players);
                if list.footer != footer {
                    list.footer = footer;
                }
            });

        world
            .system_named::<(&Compose, &mut TabList)>("tab_list_sync")
            .kind(id::<flecs::pipeline::OnStore>())
            .each(|(compose, list)| {
                if !list.changed() {
                    return;
                }
                let packet = TabListPacket {
                    header: list.header.to_tag(),
                    footer: list.footer.to_tag(),
                };
                let sent = compose
                    .broadcast(Clientbound::new(PacketId::TabList.to_raw(), &packet))
                    .send();
                if let Err(error) = sent {
                    error!("failed to broadcast the tab list: {error}");
                    return;
                }
                // Recorded only on a send that worked, so a failed encode is
                // retried next tick rather than remembered as delivered.
                list.sent = Some((list.header.clone(), list.footer.clone()));
            });

        // A change-only broadcast is invisible to anyone who was not connected
        // when the change happened, so a joining client is handed the current
        // text. `Channel` is added at the end of `enter_world`, which is after
        // the roster goes out, so this rides along with the rest of the join
        // burst.
        world
            .observer_named::<flecs::OnAdd, ()>("tab_list_on_join")
            .with(id::<Channel>())
            .each_entity(|entity, ()| {
                let Some(connection_id) = entity.try_get::<&ConnectionId>(|id| *id) else {
                    return;
                };
                entity
                    .world()
                    .get::<(&Compose, &TabList)>(|(compose, list)| {
                        if let Err(error) = unicast(compose, list, connection_id) {
                            error!("failed to send the tab list to a joining player: {error}");
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tick has no rate in it, and neither has any number of ticks over
    /// less than a window.
    #[test]
    fn a_partial_window_reports_nothing_rather_than_a_guess() {
        let mut tps = Tps::default();
        let start = Instant::now();
        tps.absorb(start);
        assert_eq!(tps.rate, None);
        assert_eq!(
            footer_readout(tps.rate, 20.0, 0).plain(),
            "TPS sampling\n0 players online"
        );

        // Four seconds of perfect ticking is still not a window.
        for tick in 1..=80 {
            tps.absorb(start + Duration::from_millis(50 * tick));
        }
        assert_eq!(tps.rate, None);
    }

    /// A server holding the pace prints the pace, and prints it *because* it
    /// counted a hundred intervals, not because 20.0 was typed anywhere.
    #[test]
    fn a_server_keeping_up_reads_exactly_the_target() {
        let mut tps = Tps::default();
        let start = Instant::now();
        for tick in 0..=200 {
            tps.absorb(start + Duration::from_millis(50 * tick));
        }
        assert_eq!(tps.rate, Some(20.0));
        assert_eq!(
            footer_readout(tps.rate, 20.0, 3).plain(),
            "TPS 20.0 / 20.0\n3 players online"
        );
    }

    /// The number the whole feature exists for: a server that fell behind says
    /// so. Ten ticks a second is half the pace, and the label prints the half
    /// beside the whole rather than normalising one away.
    #[test]
    fn a_server_falling_behind_prints_what_it_managed() {
        let mut tps = Tps::default();
        let start = Instant::now();
        for tick in 0..=100 {
            tps.absorb(start + Duration::from_millis(100 * tick));
        }
        assert_eq!(tps.rate, Some(10.0));
        assert_eq!(
            footer_readout(tps.rate, 20.0, 1).plain(),
            "TPS 10.0 / 20.0\n1 player online"
        );
    }

    /// The window slides: a burst of slow ticks ages out and the reading
    /// recovers, rather than being held down by a stall that is over.
    #[test]
    fn the_window_forgets_a_stall_once_it_is_out_of_range() {
        let mut tps = Tps::default();
        let start = Instant::now();
        // Six seconds at half pace.
        for tick in 0..=60 {
            tps.absorb(start + Duration::from_millis(100 * tick));
        }
        assert_eq!(tps.rate, Some(10.0));

        // Then six seconds at full pace, which is more than one window, so
        // nothing from the slow stretch is left in it.
        let recovered = start + Duration::from_millis(6000);
        for tick in 1..=120 {
            tps.absorb(recovered + Duration::from_millis(50 * tick));
        }
        assert_eq!(tps.rate, Some(20.0));
    }

    /// Real ticks are not evenly spaced, and this is the test that says so.
    ///
    /// Every other test here feeds exact multiples of 50 ms, which quietly
    /// makes one entry land *exactly* on the window boundary and survive the
    /// pop -- so the retained span comes out at exactly [`WINDOW`]. A real
    /// server jitters, no entry lands on the boundary, the retained span is
    /// always a hair under a window, and a readiness check written against
    /// that span never fires. That shipped: the gate showed `TPS sampling`
    /// with `n=83 span=4988ms` for twenty seconds while six unit tests passed.
    ///
    /// So the fixture here is deliberately irregular, and the assertion is the
    /// one those six could not make: that a rate appears at all.
    #[test]
    fn a_server_whose_ticks_jitter_still_reports_a_rate() {
        let mut tps = Tps::default();
        let start = Instant::now();

        // A small LCG, so the offsets neither repeat on a period that divides
        // the window nor land on a whole millisecond boundary pattern.
        let mut seed = 12_345_u64;
        let mut at = start;
        let mut ticks = 0_u32;
        for _ in 0..400 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let jitter = u64::from((seed >> 33) as u32 % 9_000);
            at += Duration::from_micros(46_000 + jitter);
            tps.absorb(at);
            ticks += 1;
            if ticks > 120 {
                assert!(
                    tps.rate.is_some(),
                    "after {ticks} jittered ticks over {:?} there is still no rate",
                    at.duration_since(start)
                );
            }
        }

        // ~50.5 ms a tick on average, so a shade under 20.
        let rate = tps.rate.expect("a rate after 400 ticks");
        assert!(
            (19.0..=20.5).contains(&rate),
            "{rate} tps is not what a 46-55 ms tick produces"
        );
    }

    /// The window is bounded, so a server that runs for a week holds a hundred
    /// timestamps and not a week of them.
    #[test]
    fn the_window_does_not_grow_without_bound() {
        let mut tps = Tps::default();
        let start = Instant::now();
        for tick in 0..=20_000 {
            tps.absorb(start + Duration::from_millis(50 * tick));
        }
        assert_eq!(tps.ticks.len(), 101);
    }

    /// Nothing is sent for a tick that changed nothing, which is what stops
    /// this being the per-tick broadcast it replaces.
    #[test]
    fn an_unchanged_label_is_not_resent() {
        let mut list = TabList::default();
        assert!(
            list.changed(),
            "a client that has been told nothing needs telling"
        );

        list.sent = Some((list.header.clone(), list.footer.clone()));
        assert!(!list.changed());

        list.footer = footer_readout(Some(19.8), 20.0, 2);
        assert!(list.changed());
    }
}
