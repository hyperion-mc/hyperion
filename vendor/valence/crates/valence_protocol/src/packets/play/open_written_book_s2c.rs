use crate::{Decode, DecodeBytesAuto, Encode, Hand, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct OpenWrittenBookS2c {
    pub hand: Hand,
}
