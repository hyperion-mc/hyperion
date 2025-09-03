use crate::{Decode, DecodeBytesAuto, Encode, Hand, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct HandSwingC2s {
    pub hand: Hand,
}
