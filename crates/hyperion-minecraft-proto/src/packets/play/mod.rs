//! Play state: everything after the world is loaded.
//!
//! 89 of the 141 clientbound bodies and most serverbound ones are generated
//! from `protocol.json` into [`clientbound`] and [`serverbound`]. The rest
//! branch on a runtime value somewhere in their codec -- a registry-or-inline
//! holder, a bitset-driven optional, a length the codec derives rather than
//! writes -- which is exactly the shape the mechanical generator refuses to
//! guess at. Those are hand-written here, grouped by what they are for:
//!
//! - [`chunk`] terrain and lighting
//! - [`entity`] spawning, movement and tracked data
//! - [`player`] the player list, chat, abilities and status
//! - [`inventory`] containers, equipment and held items
//!
//! Each hand-written body names the codec it was read from, so a reader can
//! check it against the same decompiled source the generator used.

pub mod chunk;
pub mod entity;
pub mod inventory;
pub mod player;

/// Packets the server sends.
pub mod clientbound {
    include!(concat!(env!("OUT_DIR"), "/packets/play_clientbound.rs"));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(env!("OUT_DIR"), "/packets/play_serverbound.rs"));
}
