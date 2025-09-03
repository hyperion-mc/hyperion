use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt, Velocity};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntityVelocityUpdateS2c {
    pub entity_id: VarInt,
    pub velocity: Velocity,
}
