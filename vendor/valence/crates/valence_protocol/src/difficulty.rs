use crate::{Decode, DecodeBytesAuto, Encode};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}
