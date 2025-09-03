use uuid::Uuid;
use valence_bytes::CowBytes;

use crate::{Bounded, DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct PlayerSessionC2s<'a> {
    pub session_id: Uuid,
    // Public key
    pub expires_at: i64,
    pub public_key_data: Bounded<CowBytes<'a>, 512>,
    pub key_signature: Bounded<CowBytes<'a>, 4096>,
}
