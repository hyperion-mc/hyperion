//! The seam between the game and whatever Minecraft server is hosting it.
//!
//! Reads are deliberately absent from this trait. The adapter mirrors position,
//! rotation and ground state into components once per tick, so the per-tick hot
//! paths (knockback integration, arena bounds, ability cooldowns) do plain
//! component reads with no dynamic dispatch. Only writes cross the seam, and
//! writes are rare: they happen on hit, on death, on kit change, not per entity
//! per tick.

use std::sync::Arc;

use flecs_ecs::prelude::*;
use glam::Vec3;
pub use hyperion_minecraft_proto::text::{
    Component, Decoration, NamedColor, Rgb24, Run, Style, TextColor,
};

pub mod mock;

/// A piece of text the game hands to the host.
///
/// `'static` because the adapter's queue outlives the call that filled it by up
/// to a tick, so a line cannot borrow the row it was built from.
///
/// Every text-carrying method below takes one of these and none of them takes
/// a `&str`. That is the seam's job here: a colour, a weight or an italic is a
/// field on the component, so there is no way to smuggle one across as markup
/// inside a literal and have the client draw the markup instead. The sidebar
/// shipped `"[green] Emerald_Explorer 4"` to real players for exactly as long
/// as this signature said `&str`.
pub type Text = Component<'static>;

/// A sidebar row's number.
///
/// Every row has one whether or not it wants one, because the client sorts the
/// panel on it: rows go down the screen in descending order of this value, and
/// a row cannot opt out of having a rank. What it can opt out of is having the
/// number drawn, which is the difference between the two variants. Making that
/// a choice between two variants rather than a `bool` beside an `i32` is what
/// stops a row from being built with a rank it does not mean to show and
/// showing it anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Score {
    /// Drawn right-aligned against the panel edge, and sorted on. For a row
    /// where the number is the point, like a player's remaining lives.
    Shown(i32),
    /// Sorted on and not drawn. For a row whose position is meaningful and
    /// whose number is not, like a status line.
    Rank(i32),
}

impl Score {
    /// The value the client sorts on.
    #[must_use]
    pub const fn value(self) -> i32 {
        match self {
            Self::Shown(value) | Self::Rank(value) => value,
        }
    }

    /// The number as the client draws it, or `None` when it is not drawn.
    #[must_use]
    pub const fn drawn(self) -> Option<i32> {
        match self {
            Self::Shown(value) => Some(value),
            Self::Rank(_) => None,
        }
    }
}

/// One row of the sidebar.
///
/// The score is explicit rather than derived from the row's position, because
/// the client is what draws and sorts it. Anything the score already says does
/// not also belong in [`text`](Self::text).
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarLine {
    /// What the row says, drawn from the left.
    pub text: Text,
    /// The number drawn right-aligned against the panel edge, and the key the
    /// client orders rows by.
    pub score: Score,
}

/// A player as the host server knows them.
///
/// Opaque to the game. The adapter maps this onto whatever the host uses; for
/// hyperion that is the raw bits of an `Entity`.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u64);

/// Which hotbar slot an item sits in, and what it looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotbarItem {
    pub slot: u8,
    /// Vanilla item id, e.g. `minecraft:iron_axe`. Kits bind abilities to the
    /// item in the slot exactly as Mineplex did.
    pub item: &'static str,
    pub name: String,
    pub lore: Vec<String>,
}

/// A one-shot burst of particles. Purely cosmetic, so the game never branches
/// on whether it succeeded.
///
/// Sound used to travel on this enum too, which is why every ability that
/// wanted to be heard reached for [`Self::Explosion`] and the game had four
/// noises for fifty-one abilities. Audio is now [`Sound`], which carries the
/// vanilla sound event itself rather than a name something else has to map, so
/// what is left here is only the particle: three shapes, each of which really
/// is one of three things a client can be shown.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Cue {
    Explosion,
    Teleport,
    Death,
}

/// Which of the listener's volume sliders governs a sound.
///
/// A player who has turned monsters down should hear a Wither Skull quieter and
/// their own countdown unchanged, and the only thing that can arrange that is
/// the server naming the right category. Mirrors the vanilla `SoundSource`
/// ordinals; the adapter is what turns one into the other.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum SoundCategory {
    #[default]
    Master,
    Weather,
    Blocks,
    /// Monsters. Most kit abilities are this.
    Hostile,
    /// Passive and neutral mobs.
    Neutral,
    /// Other players, and a weapon connecting.
    Players,
    Ambient,
    /// Feedback rather than a thing in the world: a countdown, a result.
    Ui,
}

/// One sound, exactly as it goes on the wire.
///
/// A vanilla sound event id and nothing else, because every sound this game
/// plays has to be one a client already owns. Shipping an id the client does
/// not know is silence with no error anywhere, so `tests/sound.rs` holds every
/// id in the game against the generated `minecraft:sound_event` registry.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Sound {
    /// A vanilla sound event id, e.g. `minecraft:entity.blaze.shoot`.
    pub id: &'static str,
    pub category: SoundCategory,
    /// Loudness at the source. Also the range: a client culls a sound past
    /// `16 * volume` blocks and attenuates linearly to nothing there, so this
    /// is how far away a hit can be felt as much as how loud it is.
    pub volume: f32,
    /// Playback speed. The client clamps it to `0.5..=2.0`.
    pub pitch: f32,
}

impl Sound {
    /// A sound at its natural volume and pitch.
    #[must_use]
    pub const fn new(id: &'static str, category: SoundCategory) -> Self {
        Self {
            id,
            category,
            volume: 1.0,
            pitch: 1.0,
        }
    }

    #[must_use]
    pub const fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }

    #[must_use]
    pub const fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }
}

/// Where a message goes on screen.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Channel {
    Chat,
    ActionBar,
    Title,
}

/// Everything the game asks the host server to do.
///
/// Kept small on purpose. Anything that can be computed from
/// mirrored components is computed in the game instead of being asked for here,
/// because every method on this trait is a wiring task later and a virtual call
/// now.
pub trait Server: Send + Sync + 'static {
    /// Add to a player's velocity. This is how knockback reaches the client;
    /// the game owns the magnitude, the host owns the physics.
    fn add_velocity(&self, player: PlayerId, delta: Vec3);

    fn teleport(&self, player: PlayerId, to: Vec3);

    /// Push the game's authoritative health onto the client's health bar.
    fn set_health(&self, player: PlayerId, health: f32, max: f32);

    /// Replace the hotbar wholesale. Called on kit selection and on respawn.
    fn set_hotbar(&self, player: PlayerId, items: &[HotbarItem]);

    fn send_message(&self, player: PlayerId, channel: Channel, text: Text);

    fn broadcast(&self, channel: Channel, text: Text);

    /// Replace a player's sidebar. `lines` is top to bottom.
    fn set_sidebar(&self, player: PlayerId, title: Text, lines: &[SidebarLine]);

    /// Toggle spectator mode: invisible, non-colliding, flying, no attacks.
    fn set_spectating(&self, player: PlayerId, spectating: bool);

    fn cue(&self, at: Vec3, cue: Cue);

    /// Play a sound at a point in the world. Everyone close enough hears it,
    /// attenuated by how far they are standing from `at`.
    fn play_sound(&self, at: Vec3, sound: Sound);

    /// Play a sound only `player` hears, at their own ears, so it is the same
    /// loudness wherever they are standing.
    ///
    /// For feedback that is about the match rather than about a place: a
    /// countdown tick has no position, and playing one in the world would make
    /// it quieter for whoever happened to be furthest from the origin.
    fn play_sound_to(&self, player: PlayerId, sound: Sound);
}

/// Singleton holding the live [`Server`].
///
/// Registered with [`flecs::Singleton`] so systems can name `&ServerHandle` as
/// an ordinary query term and have flecs resolve it once per table rather than
/// doing a world lookup per entity.
#[derive(Component)]
pub struct ServerHandle(pub Arc<dyn Server>);

impl ServerHandle {
    pub fn new(server: impl Server) -> Self {
        Self(Arc::new(server))
    }
}

impl core::ops::Deref for ServerHandle {
    type Target = dyn Server;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}
