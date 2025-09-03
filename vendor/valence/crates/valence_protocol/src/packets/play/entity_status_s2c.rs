use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct EntityStatusS2c {
    pub entity_id: i32,
    pub entity_status: u8,
}
