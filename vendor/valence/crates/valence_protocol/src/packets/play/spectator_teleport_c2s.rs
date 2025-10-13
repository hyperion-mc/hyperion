use uuid::Uuid;

use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct SpectatorTeleportC2s {
    pub target: Uuid,
}
