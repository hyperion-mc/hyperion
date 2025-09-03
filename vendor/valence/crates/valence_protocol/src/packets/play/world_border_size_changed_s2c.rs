use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct WorldBorderSizeChangedS2c {
    pub diameter: f64,
}
