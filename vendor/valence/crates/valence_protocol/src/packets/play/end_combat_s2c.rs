use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

/// Unused by notchian clients.
#[derive(Copy, Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EndCombatS2c {
    pub duration: VarInt,
}
