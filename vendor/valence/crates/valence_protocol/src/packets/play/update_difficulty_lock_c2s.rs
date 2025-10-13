use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct UpdateDifficultyLockC2s {
    pub locked: bool,
}
