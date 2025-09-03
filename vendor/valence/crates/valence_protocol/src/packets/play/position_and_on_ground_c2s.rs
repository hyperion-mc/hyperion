use valence_math::DVec3;

use crate::{packet_id, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::POSITION_AND_ON_GROUND)]
pub struct PositionAndOnGroundC2s {
    pub position: DVec3,
    pub on_ground: bool,
}
