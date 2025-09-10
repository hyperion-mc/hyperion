use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::LOOK_AND_ON_GROUND)]
pub struct LookAndOnGroundC2s {
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}
