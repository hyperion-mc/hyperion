#![expect(missing_docs, reason = "todo: fix")]
//! Protocol messages for communication between proxy and server components.
//!
//! This crate defines the message types and serialization formats used for
//! communication between the proxy and server components of the system.
//!
//! The messages are divided into:
//! - Proxy to server messages ([`proxy_to_server`])
//! - Server to proxy messages ([`server_to_proxy`])
//! - Shared message types ([`shared`])

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
