//! Play state: everything after the world is loaded.

/// Packets the server sends.
pub mod clientbound {
    include!(concat!(env!("OUT_DIR"), "/packets/play_clientbound.rs"));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(env!("OUT_DIR"), "/packets/play_serverbound.rs"));
}
