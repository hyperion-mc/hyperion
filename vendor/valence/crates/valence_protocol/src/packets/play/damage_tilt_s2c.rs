use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct DamageTiltS2c {
    /// The ID of the entity taking damage.
    pub entity_id: VarInt,
    /// The direction the damage is coming from in relation to the entity.
    pub yaw: f32,
}
