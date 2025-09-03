use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntityAttachS2c {
    pub attached_entity_id: i32,
    pub holding_entity_id: i32,
}
