use std::io::Write;

use glam::Vec3;
use hyperion_minecraft_proto::{
    Holder,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::clientbound,
    types::{SoundEventDirect, SoundSource},
};

use crate::{PacketBundle, net::protocol::Clientbound};

/// A positioned sound, ready to send to any number of players.
#[must_use]
pub struct Sound {
    id: valence_ident::Ident,
    /// Position in eighths of a block, which is the fixed point
    /// `ClientboundSoundPacket` uses.
    position: glam::IVec3,
    volume: f32,
    pitch: f32,
    seed: i64,
}

#[must_use]
pub struct SoundBuilder {
    position: Vec3,
    pitch: f32,
    volume: f32,
    seed: Option<i64>,
    sound: valence_ident::Ident,
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
            id: self.sound,
            // `ClientboundSoundPacket` writes `(int) (x * 8.0)`, so the client
            // resolves a sound to an eighth of a block and no finer.
            position: (self.position * 8.0).as_ivec3(),
            volume: self.volume,
            pitch: self.pitch,
            // A seed the server does not choose is one the client cannot
            // reproduce, which is what makes two players hear the same
            // variant of a multi-sample sound.
            seed: self.seed.unwrap_or_else(|| fastrand::i64(..)),
        }
    }
}

impl PacketBundle for &Sound {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()> {
        let body = clientbound::Sound {
            // Inline rather than a registry id: hyperion sends the vanilla
            // registries by name alone, so it has no id table to look a sound
            // up in, and the inline form is what a name-only server has.
            sound: Holder::Inline(SoundEventDirect {
                location: self.id.as_str(),
                // `None` means the client uses the sound's own attenuation
                // distance rather than a server override.
                fixed_range: None,
            }),
            source: SoundSource::Master,
            x: self.position.x,
            y: self.position.y,
            z: self.position.z,
            volume: self.volume,
            pitch: self.pitch,
            seed: self.seed,
        };
        Clientbound::new(PacketId::Sound.to_raw(), &body).encode_including_ids(w)
    }
}

pub const fn sound(sound: valence_ident::Ident, position: Vec3) -> SoundBuilder {
    SoundBuilder {
        position,
        pitch: 1.0,
        volume: 1.0,
        seed: None,
        sound,
    }
}
