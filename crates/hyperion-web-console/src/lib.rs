//! An operator's window onto a running hyperion server, in a browser.
//!
//! Watch chat as it happens, say something back, and run any command a player
//! could run -- as an operator, from outside the game, without a Minecraft
//! client. The page is styled like the game because that is what the thing it
//! shows looks like: section-sign colours are rendered rather than stripped, so
//! a line reads on the web the way it reads in the chat box.
//!
//! # Why this is engine-level
//!
//! Watching and administering a server is not a game concept. `smash` and
//! `bedwars` are two games on one engine and an operator's question --- who is
//! on, is it keeping up, what build is this, say something to everybody --- is
//! the same question for both. So nothing here knows what game is running:
//! chat is taken at the point hyperion decodes it, the roster is hyperion's own
//! [`Ping`](hyperion::egress::ping::Ping) and [`Name`], and commands go through
//! the registry every event already registers into. The precedent is
//! `hyperion::egress::server_load`, which is telemetry in the engine for the
//! same reason.
//!
//! # Commands run through the same dispatch players use
//!
//! There is no admin API here. A command typed on the web becomes
//! [`event::Command`], the same event a `chat_command` packet produces, and is
//! executed by `hyperion_command`'s own system against the same
//! [`CommandRegistry`](hyperion_command::CommandRegistry). One implementation
//! of `/perms`, not two.
//!
//! What that costs is the console needing an identity, because a command reads
//! its caller's permission group and answers to its caller's connection. So the
//! module makes one entity --- named `Console`, group `Admin` --- and gives it a
//! [`ConnectionId`](hyperion::net::ConnectionId) with no socket behind it,
//! registered through [`hyperion::net::VirtualConnection`]. Replies addressed
//! to it are intercepted before the proxy and decoded back into text for the
//! page, using the same [`FrameDecoder`](hyperion_minecraft_proto::framing::FrameDecoder)
//! a client would. Every command in the workspace answers
//! `caller.get::<&ConnectionId>()`; teaching all of them a second kind of reply
//! would have been the alternative, and it would have been a worse one.
//!
//! # Authentication, and what it is not
//!
//! One bearer token, read from a file at startup, compared in constant time.
//! No accounts, no sessions, no login page. The bind address defaults to
//! loopback and the NixOS option that exposes it is off by default, because a
//! console is an admin surface and the honest default for an admin surface is
//! "reachable from the box it runs on". See [`http`] for where the token
//! travels and the one place it is not in a header.

pub mod feed;
pub mod http;
pub mod legacy;
pub mod module;
pub mod state;
mod virtual_connection;

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

pub use module::{ConsoleComponentsModule, ConsoleModule, install};

use crate::{
    feed::Feed,
    state::{Inbox, Request, Snapshot},
};

/// How the console was told to run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Where to listen. Loopback unless an operator says otherwise.
    pub address: SocketAddr,
    /// The shared secret every request must carry.
    pub token: String,
}

/// Everything the web server and the tick loop share.
///
/// One `Arc`, handed to both. The web server never touches the world and the
/// tick loop never touches a socket; [`Inbox`] and [`Snapshot`] are the two
/// places they meet, and both are plain mutexes over small values written once
/// a tick at most.
#[derive(Debug)]
pub struct Console {
    pub feed: Feed,
    inbox: Inbox,
    snapshot: Mutex<Snapshot>,
    token: String,
}

impl Console {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            feed: Feed::new(),
            inbox: Inbox::default(),
            snapshot: Mutex::new(Snapshot::default()),
            token,
        }
    }

    /// Whether `offered` is the configured token.
    ///
    /// Constant time in the length of the token, so a caller cannot learn it
    /// one byte at a time from how long the comparison took. An empty token is
    /// refused outright rather than matching an empty configuration, because
    /// the failure mode of the other choice is a console with no password that
    /// looks like it has one.
    #[must_use]
    pub fn authorises(&self, offered: &[u8]) -> bool {
        if self.token.is_empty() || offered.is_empty() {
            return false;
        }
        let expected = self.token.as_bytes();
        if expected.len() != offered.len() {
            return false;
        }
        let mut difference = 0_u8;
        for (left, right) in expected.iter().zip(offered) {
            difference |= left ^ right;
        }
        difference == 0
    }

    /// Queue something for the next tick to carry out.
    pub fn submit(&self, request: Request) {
        self.inbox.push(request);
    }

    /// Everything queued since the last tick.
    #[must_use]
    pub fn take_requests(&self) -> Vec<Request> {
        self.inbox.drain()
    }

    /// The state the page draws, as JSON.
    #[must_use]
    pub fn snapshot(&self) -> String {
        self.snapshot
            .lock()
            .map_or_else(|_unused| "{}".to_owned(), |snapshot| snapshot.to_json())
    }

    /// Replace the state the page draws.
    pub fn publish(&self, snapshot: Snapshot) {
        if let Ok(mut held) = self.snapshot.lock() {
            *held = snapshot;
        }
    }
}

/// Start the web server on its own runtime.
///
/// Its own, rather than the one hyperion runs the proxy connection on: an
/// operator console must not be able to starve the tick loop's I/O, and a
/// single worker thread is more than a page of text needs.
///
/// # Errors
/// Fails if the runtime cannot be built or the address cannot be bound. Both
/// are startup failures an operator has to hear about: a console that silently
/// did not come up is indistinguishable from one nobody opened. Everything
/// after the bind is a per-connection failure and is logged rather than
/// returned, because there is nobody left to return it to.
pub fn spawn(console: Arc<Console>, address: SocketAddr) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Bound here and handed over, rather than bound here and bound again in
    // the thread. Binding twice was the first version of this and it is two
    // bugs: the port is released between the two calls, so something else can
    // take it and the second bind fails for a reason the first one proved was
    // not true; and on a platform with `SO_REUSEADDR` semantics that permit it,
    // both binds succeed and the console serves on a socket nobody checked.
    let listener = runtime.block_on(async { tokio::net::TcpListener::bind(address).await })?;

    std::thread::Builder::new()
        .name("hyperion-console".to_owned())
        .spawn(move || {
            runtime.block_on(async move {
                http::serve(console, listener).await;
            });
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Console;

    #[test]
    fn the_right_token_is_accepted_and_a_wrong_one_is_not() {
        let console = Console::new("s3cret".to_owned());
        assert!(console.authorises(b"s3cret"));
        assert!(!console.authorises(b"s3cres"));
        assert!(!console.authorises(b"s3cret "));
    }

    #[test]
    fn an_empty_token_never_matches() {
        // The dangerous shape: a console started with no token configured,
        // answering every request that also sent nothing.
        assert!(!Console::new(String::new()).authorises(b""));
        assert!(!Console::new("s3cret".to_owned()).authorises(b""));
    }
}
