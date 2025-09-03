use crate::{Decode, DecodeBytesAuto, Difficulty, Encode, Packet};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct UpdateDifficultyC2s {
    pub difficulty: Difficulty,
}
