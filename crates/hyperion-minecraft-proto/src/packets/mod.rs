//! Packet bodies.
//!
//! Layouts are transcribed from the decompiled server sources for this
//! protocol version. Each packet carries the name of the class it came from so
//! a reader can check it against the jar.
//!
//! Handshake, status, login and configuration are complete. [`play_login`]
//! covers only the join sequence; the rest of play is not implemented. See
//! `docs/minecraft-26.2-migration.md` for what remains.

pub mod configuration;
pub mod handshake;
pub mod login;
pub mod play_login;
pub mod status;
