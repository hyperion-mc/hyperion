use crate::{Decode, DecodeBytesAuto, Encode, Packet};

/// Unused by notchian clients.
#[derive(Copy, Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EnterCombatS2c;
