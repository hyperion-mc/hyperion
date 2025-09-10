use valence_math::DVec3;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::POSITION_AND_ON_GROUND)]
pub struct PositionAndOnGroundC2s {
    pub position: DVec3,
    pub on_ground: bool,
}
