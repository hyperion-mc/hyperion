use valence_math::IVec3;

use crate::sound::{SoundCategory, SoundId};
use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct PlaySoundS2c {
    pub id: SoundId,
    pub category: SoundCategory,
    pub position: IVec3,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}
