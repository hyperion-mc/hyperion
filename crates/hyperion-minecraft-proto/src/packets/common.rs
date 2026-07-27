//! Packets more than one state sends.
//!
//! `net.minecraft.network.protocol.common` and its neighbours hold packet
//! classes that several protocol states register. One Java class is one Rust
//! type here too, re-exported into each state that sends it, so a value built
//! for one state is the same value in another.

/// Packets the server sends.
pub mod clientbound {
    include!(concat!(env!("OUT_DIR"), "/packets/common_clientbound.rs"));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(env!("OUT_DIR"), "/packets/common_serverbound.rs"));
}
