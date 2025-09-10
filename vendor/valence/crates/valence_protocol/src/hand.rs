use crate::{Decode, DecodeBytesAuto, Encode};

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum Hand {
    #[default]
    Main,
    Off,
}
