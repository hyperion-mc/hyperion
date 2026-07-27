//! Login state: authentication and the handover into configuration.

/// Packets the server sends.
pub mod clientbound {
    include!(concat!(env!("OUT_DIR"), "/packets/login_clientbound.rs"));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(env!("OUT_DIR"), "/packets/login_serverbound.rs"));
}
