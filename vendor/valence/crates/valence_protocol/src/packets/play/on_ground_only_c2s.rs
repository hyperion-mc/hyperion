use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::ON_GROUND_ONLY)]
pub struct OnGroundOnlyC2s {
    pub on_ground: bool,
}
