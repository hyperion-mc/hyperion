//! What the console has seen, and everyone watching it.

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::broadcast;

/// How many lines a browser is handed when it connects.
///
/// A console opened after something went wrong is the normal case, so the
/// backlog is what makes the page useful at all rather than a nicety. Bounded
/// because this is a live server's memory: two hundred lines is a screenful
/// several times over and costs a few tens of kilobytes.
pub const BACKLOG: usize = 200;

/// How many lines a slow browser may fall behind before it is cut loose.
///
/// A dropped subscriber is told how many it missed rather than silently handed
/// a gap; see [`Feed::subscribe`].
const CHANNEL_CAPACITY: usize = 512;

/// Where a line came from, which is the only thing the page styles on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A player typed it.
    Chat,
    /// The console typed it, and every player saw it.
    Console,
    /// A reply addressed to the console: the output of a command it ran.
    Reply,
    /// The server saying something about itself. A join, a leave, a refusal.
    System,
}

impl Source {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Console => "console",
            Self::Reply => "reply",
            Self::System => "system",
        }
    }
}

/// One line, as the page will draw it.
///
/// The text keeps its section-sign codes. Rendering them is the browser's job
/// -- it is the only place that knows what a colour looks like -- and stripping
/// them here would throw away the thing that makes the page look like the game.
#[derive(Debug, Clone)]
pub struct Line {
    /// Monotonic within a process, so a page can tell it has the whole story.
    /// Not a timestamp: two lines in one tick share a millisecond.
    pub seq: u64,
    /// Unix milliseconds, for the clock the page draws.
    pub at: u64,
    pub source: Source,
    pub text: String,
}

impl Line {
    /// This line as one SSE `data:` payload.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "seq": self.seq,
            "at": self.at,
            "source": self.source.as_str(),
            "text": self.text,
        })
        .to_string()
    }
}

/// Every line the console has, and a tap for anyone watching.
#[derive(Debug)]
pub struct Feed {
    next_seq: AtomicU64,
    backlog: Mutex<VecDeque<Line>>,
    live: broadcast::Sender<Line>,
}

impl Default for Feed {
    fn default() -> Self {
        Self::new()
    }
}

impl Feed {
    #[must_use]
    pub fn new() -> Self {
        let (live, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            next_seq: AtomicU64::new(1),
            backlog: Mutex::new(VecDeque::with_capacity(BACKLOG)),
            live,
        }
    }

    /// Record a line and hand it to everyone watching.
    pub fn push(&self, source: Source, text: impl Into<String>) {
        let line = Line {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            at: now_ms(),
            source,
            text: text.into(),
        };

        if let Ok(mut backlog) = self.backlog.lock() {
            if backlog.len() == BACKLOG {
                backlog.pop_front();
            }
            backlog.push_back(line.clone());
        }

        // `Err` means nobody is watching, which is the usual state of an
        // operator console and not a problem.
        let _unused = self.live.send(line);
    }

    /// The lines a page gets before the live ones start.
    #[must_use]
    pub fn backlog(&self) -> Vec<Line> {
        self.backlog
            .lock()
            .map(|backlog| backlog.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// A live tap. Every subscriber gets every line pushed after this call.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Line> {
        self.live.subscribe()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::{BACKLOG, Feed, Source};

    #[test]
    fn the_backlog_is_bounded_and_keeps_the_newest() {
        let feed = Feed::new();
        for index in 0..(BACKLOG + 10) {
            feed.push(Source::System, format!("line {index}"));
        }

        let backlog = feed.backlog();
        assert_eq!(backlog.len(), BACKLOG);
        assert_eq!(backlog[0].text, "line 10");
        assert_eq!(backlog[BACKLOG - 1].text, format!("line {}", BACKLOG + 9));
    }

    #[test]
    fn sequence_numbers_are_dense_and_increasing() {
        let feed = Feed::new();
        feed.push(Source::Chat, "one");
        feed.push(Source::Chat, "two");

        let backlog = feed.backlog();
        assert_eq!(backlog[1].seq, backlog[0].seq + 1);
    }

    #[test]
    fn a_section_sign_survives_into_the_feed() {
        // The browser renders these. Stripping them here would be the console
        // quietly deciding the page should look like a terminal.
        let feed = Feed::new();
        feed.push(Source::Console, "\u{a7}cred");
        assert_eq!(feed.backlog()[0].text, "\u{a7}cred");
    }
}
