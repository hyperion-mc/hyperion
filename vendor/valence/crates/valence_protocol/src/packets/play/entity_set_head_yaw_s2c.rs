use crate::{ByteAngle, Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntitySetHeadYawS2c {
    pub entity_id: VarInt,
    pub head_yaw: ByteAngle,
}
