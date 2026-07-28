//! Hyperion's own protocol between the game server and the proxy.
//!
//! Not the Minecraft protocol. That is `hyperion-minecraft-proto`, which
//! speaks Mojang's wire format to a real client. This crate is the private
//! link between two hyperion processes, and it never interprets a Minecraft
//! packet: player traffic crosses it as an opaque `&[u8]` that the proxy
//! forwards and the server decodes.
//!
//! Two enums carry everything, one per direction:
//! [`ProxyToServerMessage`] for connects, disconnects and inbound player
//! packets, and [`ServerToProxyMessage`] for the outbound side, which is
//! mostly the proxy's job of fanning one packet out to the right players
//! (see [`BroadcastGlobal`], [`BroadcastLocal`], [`BroadcastChannel`],
//! [`Unicast`]) plus the position and channel bookkeeping that tells it who
//! those players are.
//!
//! Encoding is rkyv, so the proxy reads an `Archived*` view straight out of
//! the received bytes without deserialising. Both ends are built from this
//! same source in one workspace, which is what makes that safe; there is no
//! version negotiation and no compatibility window.

#![allow(
    clippy::module_inception,
    clippy::module_name_repetitions,
    clippy::derive_partial_eq_without_eq,
    hidden_glob_reexports
)]

mod proxy_to_server;
mod server_to_proxy;
mod shared;

pub use proxy_to_server::*;
pub use server_to_proxy::*;
pub use shared::*;
