use crate::{Decode, DecodeBytesAuto, Difficulty, Encode, Packet};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct DifficultyS2c {
    pub difficulty: Difficulty,
    pub locked: bool,
}
