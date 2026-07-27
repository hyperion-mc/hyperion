use std::io::Write;

use glam::Vec3;
use valence_protocol::{
    packets::play,
    sound::{SoundCategory, SoundId},
};

use crate::PacketBundle;

#[must_use]
pub struct Sound {
    raw: play::PlaySoundS2c<'static>,
}

#[must_use]
pub struct SoundBuilder {
    position: Vec3,
    pitch: f32,
    volume: f32,
    seed: Option<i64>,
    sound: valence_ident::Ident<&'static str>,
}

impl SoundBuilder {
    pub const fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    pub const fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    pub const fn seed(mut self, seed: i64) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn build(self) -> Sound {
        Sound {
            raw: play::PlaySoundS2c {
                id: SoundId::Direct {
                    id: self.sound.into(),
                    range: None,
                },
                position: (self.position * 8.0).as_ivec3(),
                volume: self.volume,
                pitch: self.pitch,
                seed: self.seed.unwrap_or_else(|| fastrand::i64(..)),
                category: SoundCategory::Master,
            },
        }
    }
}

impl PacketBundle for &Sound {
    fn encode_including_ids(self, mut w: impl Write) -> anyhow::Result<()> {
        self.raw.encode_including_ids(&mut w)
    }
}

pub const fn sound(sound: valence_ident::Ident<&'static str>, position: Vec3) -> SoundBuilder {
    SoundBuilder {
        position,
        pitch: 1.0,
        volume: 1.0,
        seed: None,
        sound,
    }
}
