use valence_math::DVec3;

use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::FULL)]
pub struct FullC2s {
    pub position: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}
