use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct TitleFadeS2c {
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
}
