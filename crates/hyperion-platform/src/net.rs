//! Sockets.
//!
//! Both platforms have `std::net`, so this module does not wrap it. What it
//! does is name the two things that differ, so a call site can ask about the
//! capability instead of the operating system.
//!
//! Hermit's stack is smoltcp over virtio-net. It gives IPv4 and IPv6 TCP and
//! UDP, and nothing else: no `AF_UNIX`, and DNS only when the kernel was built
//! with its resolver.

pub use std::net::{TcpListener, TcpStream, UdpSocket};
use std::{io, net::SocketAddr};

/// Bind a TCP listener, naming the platform in the error.
///
/// A bare `bind` failure on a unikernel is an opaque `Uncategorized`, which
/// sends people looking at their firewall rather than at the fact that the
/// guest never got an address. Say which platform failed.
///
/// # Errors
/// Whatever the platform's `bind` reports.
pub fn bind_tcp(addr: SocketAddr) -> io::Result<TcpListener> {
    TcpListener::bind(addr).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("{platform}: failed to bind {addr}: {e}", platform = crate::NAME),
        )
    })
}

/// Whether `AF_UNIX` sockets exist on this platform.
///
/// The proxy prefers a Unix socket for the server link when both ends share a
/// machine. On a unikernel there is no such thing, and no filesystem to put one
/// in, so the loopback path is the only path.
#[must_use]
pub const fn supports_unix_sockets() -> bool {
    crate::CAPABILITIES.unix_sockets
}

/// Whether hostnames resolve.
///
/// When this is `false`, every peer address must already be a literal.
#[must_use]
pub const fn supports_dns() -> bool {
    crate::CAPABILITIES.dns
}
