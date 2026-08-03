//! What the console shows besides chat, and what it sends back into the world.

use std::sync::Mutex;

/// One player, as the console's roster draws them.
#[derive(Debug, Clone)]
pub struct PlayerRow {
    pub name: String,
    /// Round trip in milliseconds, or `None` when nothing has been measured.
    ///
    /// `None` rather than a zero, for the same reason
    /// [`hyperion::egress::ping::UNKNOWN`] exists: a zero draws a healthy
    /// connection for a client nobody has timed.
    pub latency_ms: Option<i32>,
}

/// Everything the page shows that is not a line of chat.
///
/// Rebuilt whole once a second by a flecs system and read by the web server on
/// whatever thread it happens to be on, so it crosses a mutex rather than
/// being queried live: a query needs the world, and the world belongs to the
/// tick loop.
#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub players: Vec<PlayerRow>,
    /// Ticks per second over the engine's own window, or `None` while it is
    /// still sampling. Not defaulted to twenty: a number nobody has measured
    /// is exactly what an operator is checking for.
    pub tps: Option<f32>,
    /// What the tick loop is paced to, so the rate above has a denominator.
    pub target_tps: f32,
    /// The commit this server was built from, `None` when nothing said.
    pub build_rev: Option<String>,
    /// The working tree had uncommitted changes at build time.
    pub build_dirty: bool,
}

impl Snapshot {
    /// This snapshot as the JSON the page reads.
    #[must_use]
    pub fn to_json(&self) -> String {
        let players: Vec<_> = self
            .players
            .iter()
            .map(|player| serde_json::json!({ "name": player.name, "latency": player.latency_ms }))
            .collect();

        serde_json::json!({
            "players": players,
            "tps": self.tps,
            "targetTps": self.target_tps,
            "buildRev": self.build_rev,
            "buildDirty": self.build_dirty,
        })
        .to_string()
    }
}

/// What the operator asked for, waiting for a tick to carry it out.
///
/// A web request arrives on a tokio thread and everything it wants to do needs
/// the world, which only the tick loop may touch. So a request lands here and
/// the next tick drains it. The operator's own latency is one tick, 50 ms,
/// which is under what a click can perceive.
#[derive(Debug, Clone)]
pub enum Request {
    /// Say something to every player, as the console.
    Say(String),
    /// Run a command, as the console operator.
    Command(String),
}

/// The queue those requests wait in.
#[derive(Debug, Default)]
pub struct Inbox {
    pending: Mutex<Vec<Request>>,
}

impl Inbox {
    pub fn push(&self, request: Request) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(request);
        }
    }

    /// Everything queued since the last call.
    #[must_use]
    pub fn drain(&self) -> Vec<Request> {
        self.pending
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Inbox, PlayerRow, Request, Snapshot};

    #[test]
    fn draining_leaves_the_queue_empty() {
        let inbox = Inbox::default();
        inbox.push(Request::Say("hello".to_owned()));
        assert_eq!(inbox.drain().len(), 1);
        assert!(inbox.drain().is_empty());
    }

    #[test]
    fn an_unmeasured_ping_is_null_and_not_zero() {
        // A zero here draws five bars for a client nobody has timed, which is
        // the bug ENG-11113 fixed on the tab list. The console must not
        // reintroduce it in a second place.
        let snapshot = Snapshot {
            players: vec![PlayerRow {
                name: "Andrew".to_owned(),
                latency_ms: None,
            }],
            ..Snapshot::default()
        };
        assert!(snapshot.to_json().contains(r#""latency":null"#));
    }

    #[test]
    fn a_tps_still_sampling_is_null_and_not_twenty() {
        let snapshot = Snapshot {
            target_tps: 20.0,
            ..Snapshot::default()
        };
        assert!(snapshot.to_json().contains(r#""tps":null"#));
    }
}
