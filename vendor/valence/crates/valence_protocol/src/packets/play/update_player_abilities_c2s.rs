use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet, PartialEq, Eq)]
pub enum UpdatePlayerAbilitiesC2s {
    #[packet(tag = 0b00)]
    StopFlying,
    #[packet(tag = 0b10)]
    StartFlying,
}
