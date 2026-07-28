//! Positioned sound, as a packet any caller can send without naming one.
//!
//! The whole point of this module is that a game says *what* it wants heard,
//! *where*, *how loud* and *under whose volume slider*, and never touches a
//! packet id, a registry holder or a fixed point encoding. Everything below the
//! [`SoundBuilder`] is that translation.

use std::io::Write;

use glam::{I16Vec2, Vec3};
use hyperion_minecraft_proto::{
    Holder,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::clientbound,
    types::{SoundEventDirect, SoundSource},
};

use crate::{
    PacketBundle,
    net::{Compose, ConnectionId, protocol::Clientbound},
};

/// Which of the player's volume sliders governs a sound.
///
/// The client mixes every sound through the category the server names, so a
/// player who has turned Hostile Creatures down hears a Wither Skull quieter
/// and their own UI clicks unchanged. Sending everything as
/// [`Self::Master`] takes that choice away from them, which is why this is a
/// parameter rather than a constant.
///
/// Mirrors `net.minecraft.sounds.SoundSource`. It is restated here rather than
/// re-exported so that a caller names a category and not a protocol type, which
/// is the promise the rest of [`crate::net::agnostic`] makes.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum SoundCategory {
    #[default]
    Master,
    Music,
    Records,
    Weather,
    Blocks,
    /// Monsters. A mob ability belongs here.
    Hostile,
    /// Passive and neutral mobs.
    Neutral,
    /// Other players, and the swing of a weapon connecting.
    Players,
    Ambient,
    Voice,
    /// Menus, buttons and anything that is feedback rather than a thing in the
    /// world. A countdown tick is this.
    Ui,
}

impl SoundCategory {
    const fn to_source(self) -> SoundSource {
        match self {
            Self::Master => SoundSource::Master,
            Self::Music => SoundSource::Music,
            Self::Records => SoundSource::Records,
            Self::Weather => SoundSource::Weather,
            Self::Blocks => SoundSource::Blocks,
            Self::Hostile => SoundSource::Hostile,
            Self::Neutral => SoundSource::Neutral,
            Self::Players => SoundSource::Players,
            Self::Ambient => SoundSource::Ambient,
            Self::Voice => SoundSource::Voice,
            Self::Ui => SoundSource::Ui,
        }
    }
}

/// How far a sound at volume 1.0 carries, in blocks.
///
/// The client attenuates a sound linearly to nothing at `16 * volume` blocks
/// from where the server put it, so volume is a range control as much as a
/// loudness one and raising it above 1.0 is how a caller says "this should be
/// heard across the arena". Who is *sent* the packet at all is a separate and
/// coarser decision: see [`Sound::broadcast_near`].
pub const RANGE_PER_VOLUME: f32 = 16.0;

/// A positioned sound, ready to send to any number of players.
#[must_use]
pub struct Sound {
    id: valence_ident::Ident,
    /// Position in eighths of a block, which is the fixed point
    /// `ClientboundSoundPacket` uses.
    position: glam::IVec3,
    category: SoundCategory,
    volume: f32,
    pitch: f32,
    seed: i64,
}

#[must_use]
pub struct SoundBuilder {
    position: Vec3,
    pitch: f32,
    volume: f32,
    category: SoundCategory,
    seed: Option<i64>,
    sound: valence_ident::Ident,
}

impl SoundBuilder {
    /// Playback speed, and with it the perceived size of whatever made the
    /// noise. The client clamps this to `0.5..=2.0`.
    pub const fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Loudness at the source, and the range: see [`RANGE_PER_VOLUME`].
    pub const fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    /// Which of the listener's volume sliders applies.
    pub const fn category(mut self, category: SoundCategory) -> Self {
        self.category = category;
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
            category: self.category,
            volume: self.volume,
            pitch: self.pitch,
            // A seed the server does not choose is one the client cannot
            // reproduce, which is what makes two players hear the same
            // variant of a multi-sample sound.
            seed: self.seed.unwrap_or_else(|| fastrand::i64(..)),
        }
    }
}

impl Sound {
    /// Where it plays, back in blocks.
    ///
    /// Round-trips through the wire's fixed point, so this is what a client
    /// will resolve the sound to rather than what the caller asked for.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        self.position.as_vec3() / 8.0
    }

    /// The chunk the proxy centres a local broadcast on.
    fn chunk(&self) -> I16Vec2 {
        // Eighths of a block to chunks: sixteen blocks of eight eighths each.
        let chunk = self.position.div_euclid(glam::IVec3::splat(8 * 16));
        I16Vec2::new(
            i16::try_from(chunk.x).unwrap_or(0),
            i16::try_from(chunk.z).unwrap_or(0),
        )
    }

    /// Play it for everyone close enough to hear, attenuated by their distance
    /// from its position.
    ///
    /// The attenuation is the client's: it is handed a point and works out how
    /// loud that is from where the listener stands. All the server decides is
    /// who is sent the packet at all, and it decides that by chunk, not by
    /// [`RANGE_PER_VOLUME`], so a loud sound can still be culled by a listener
    /// being several chunks out. That is the same rule vanilla's own chunk
    /// tracking applies, and it is why volume is the wrong tool for reaching a
    /// player on the far side of the map: see [`Self::play_to`].
    ///
    /// # Errors
    /// If the packet cannot be encoded or queued.
    pub fn broadcast_near(&self, compose: &Compose) -> anyhow::Result<()> {
        compose.broadcast_local(self, self.chunk()).send()
    }

    /// Play it for one player and nobody else.
    ///
    /// Sent unattenuated if the caller builds it at the listener's own
    /// position, which is what a sound about the match rather than about a
    /// place wants: a countdown tick should not be quieter for whoever is
    /// standing furthest from the origin.
    ///
    /// # Errors
    /// If the packet cannot be encoded or queued.
    pub fn play_to(&self, compose: &Compose, to: ConnectionId) -> anyhow::Result<()> {
        compose.unicast(self, to)
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
            source: self.category.to_source(),
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
        category: SoundCategory::Master,
        seed: None,
        sound,
    }
}
