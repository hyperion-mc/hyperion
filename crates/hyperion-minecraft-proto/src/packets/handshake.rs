//! Handshake state: one packet, sent before anything else.
//!
//! Its layout is frozen across protocol versions, because a server has to
//! read `protocol_version` before it knows which version it is speaking.

/// Nothing is sent clientbound in this state.
pub mod clientbound {
    include!(concat!(
        env!("OUT_DIR"),
        "/packets/handshake_clientbound.rs"
    ));
}

/// Packets the client sends.
pub mod serverbound {
    include!(concat!(
        env!("OUT_DIR"),
        "/packets/handshake_serverbound.rs"
    ));
}
