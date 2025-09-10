use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct ScreenHandlerPropertyUpdateS2c {
    pub window_id: u8,
    pub property: i16,
    pub value: i16,
}
