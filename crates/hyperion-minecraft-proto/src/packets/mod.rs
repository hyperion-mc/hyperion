//! Packet bodies.
//!
//! Layouts are transcribed from the decompiled server sources for this
//! protocol version. Each packet carries the name of the class it came from so
//! a reader can check it against the jar.
//!
//! Only the pre-play states are implemented so far. See
//! `docs/minecraft-26.2-migration.md` for what the remaining states cost.

pub mod handshake;
pub mod login;
pub mod status;
