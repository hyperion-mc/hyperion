use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct WorldBorderWarningTimeChangedS2c {
    pub warning_time: VarInt,
}
