use std::borrow::Cow;

use valence_math::{DVec3, Vec3};

use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct ExplosionS2c<'a> {
    pub pos: DVec3,
    pub strength: f32,
    pub affected_blocks: Cow<'a, [[i8; 3]]>,
    pub player_motion: Vec3,
}
