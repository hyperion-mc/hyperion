//! Status state: the server list ping exchange.

/// Packets the server sends.
pub mod clientbound {
    include!(concat!(env!("OUT_DIR"), "/packets/status_clientbound.rs"));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(env!("OUT_DIR"), "/packets/status_serverbound.rs"));
}
