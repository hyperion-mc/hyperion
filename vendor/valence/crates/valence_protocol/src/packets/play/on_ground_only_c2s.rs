use crate::{packet_id, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::ON_GROUND_ONLY)]
pub struct OnGroundOnlyC2s {
    pub on_ground: bool,
}
