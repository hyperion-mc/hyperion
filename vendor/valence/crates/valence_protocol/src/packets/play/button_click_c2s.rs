use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct ButtonClickC2s {
    pub window_id: i8,
    pub button_id: i8,
}
