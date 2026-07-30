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
pub use hyperion::effects::{Effect, Shape, Status};
pub use hyperion_minecraft_proto::{
    particle::{Argb, Particle, ParticleKind},
    text::{Component, Decoration, NamedColor, Rgb24, Run, Style, TextColor},
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

/// What a client is allowed to do about flying, and the whole of the game's
/// half of the double jump.
///
/// Super Smash Mobs' mid-air jump is vanilla creative flight, abused. A player
/// with permission to fly who double-taps the jump key sends a serverbound
/// abilities packet asking to take off, and that packet is the only mid-air
/// jump input a client without a mod can produce. So the game arms the
/// permission while a jump is available, waits for the press, and answers it
/// with an impulse instead of with flight.
///
/// Neither variant leaves the player flying, and that is the point of it being
/// two states rather than the two booleans the protocol carries. A player who
/// is actually allowed to fly in a Smash match is a player who has left the
/// game, so "keep flying" is not a thing this seam can be asked for.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum Flight {
    /// A mid-air jump is available: the client may take off, and the press
    /// that would is the input the game answers.
    Armed,
    /// No mid-air jump. On the ground, or out of them.
    #[default]
    Disarmed,
}

impl Flight {
    /// Armed exactly when a jump is available.
    #[must_use]
    pub const fn armed(armed: bool) -> Self {
        if armed { Self::Armed } else { Self::Disarmed }
    }

    #[must_use]
    pub const fn is_armed(self) -> bool {
        matches!(self, Self::Armed)
    }
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

/// A particle effect the host draws. Purely cosmetic, so the game never
/// branches on whether it succeeded.
///
/// This replaced a `Cue` enum, which by the end had five variants standing in
/// for every visual in the game: an explosion, a teleport, a death, a burn and
/// a poison. Bone Explosion, Water Splash and Fish Flurry all drew
/// `Cue::Explosion`, so bones, water and fish were the same picture. The enum
/// was never really the problem: the protocol layer under it could spell five
/// particles, so five was all an ability could ask for. Now that [`Particle`]
/// is the whole registry, an ability names the one it wants and composes the
/// shape itself.
///
/// A re-export rather than a type of its own, for the same reason [`Text`] is
/// one: a second copy of a builder is a second thing to keep in step with the
/// first. It is pure data -- a particle, a point and a shape -- and lives in
/// the engine because the emitter that draws it across several ticks does.
pub type Particles = Effect<'static>;

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
///
/// The middle of the screen is deliberately absent. A title there is not a
/// line of text but a pair of them with an animation, and the protocol makes
/// the halves separable in a way that is a trap; [`Title`] is that shape and
/// [`Server::show_title`] is the only way to reach it.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Channel {
    Chat,
    ActionBar,
}

/// The experience bar, and the number over it.
///
/// Super Smash Mobs has no experience to spend, so the bar the client already
/// draws above the hotbar is free real estate, and the mode used it for
/// ability recharge. This is that bar, as the two numbers the client takes.
///
/// Level zero draws no number at all -- `Gui.renderExperienceLevel` is guarded
/// by `experienceLevel > 0` -- which is what makes "nothing to count down"
/// expressible rather than something that has to be drawn as a zero.
///
/// [`Default`] is the state a client is in before anything is sent to it: an
/// empty bar and no number.
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Experience {
    /// How full the bar is, `0.0..=1.0`.
    pub progress: f32,
    /// The number in the green box, or zero to draw none.
    pub level: i32,
}

/// How a boss bar is tinted.
///
/// The game's own vocabulary, like [`Cue`]: four bands and not the client's
/// seven colours, because what the game means is "fine", "getting dangerous",
/// "the next hit ends you" and "this is not about you". Which Minecraft colour
/// each becomes is a hosting decision and lives in the adapter.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BarColour {
    Green,
    Yellow,
    Red,
    /// Informational: a countdown, a lobby, a result. Not a warning.
    Blue,
}

/// The bar across the top of one player's screen.
///
/// Per player and not per world, which is the whole reason it can carry a
/// number that is only true of the person reading it.
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    /// Drawn centred above the bar.
    pub title: Text,
    /// How full it is, `0.0..=1.0`.
    pub progress: f32,
    pub colour: BarColour,
}

/// Which of a player's bars a [`BossBar`] is written to.
///
/// A client draws as many bars as it is sent, stacked, and the two this game
/// puts up answer questions on completely different clocks: [`Self::Hud`]
/// changes several times a second, and [`Self::Build`] is written once and is
/// then true until the process exits. Naming the slot is what lets the second
/// one exist without the first one overwriting it every tick, and what lets a
/// test say which bar it is asserting about.
///
/// Closed and small on purpose. A slot is a strip of a player's screen, and
/// this game is a fighting game: adding a variant here is a permanent decision
/// about how much of the screen is not the arena.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum BarSlot {
    /// The match bar: knockback percentage, the lobby's state, the result.
    /// See [`crate::module::hud::boss_bar`].
    Hud,
    /// What build the server is running. See
    /// [`crate::module::build_stamp`].
    Build,
}

impl BarSlot {
    /// Every slot, in the order they are laid out.
    ///
    /// The one list. [`Self::COUNT`] is its length and [`Self::index`] is a
    /// position in it, so neither can drift from it -- an earlier version wrote
    /// `COUNT = 2` and matched out the indices by hand, which a third variant
    /// would have left at 2 while `index` returned 2, and the array in the
    /// adapter would then have panicked out of bounds inside the server's
    /// `PostUpdate` on the first tick anybody wrote that slot.
    ///
    /// Adding a variant still has to add it here; Rust cannot enumerate an
    /// enum's variants on its own. What is different is the failure: a slot
    /// missing from this list has no index and says so by name, and
    /// `every_slot_is_in_all_and_indexes_into_it` in `tests/build_stamp.rs`
    /// fails before any of that reaches a player.
    pub const ALL: &'static [Self] = &[Self::Hud, Self::Build];
    /// How many slots there are, so an adapter can hold one entity per slot in
    /// an array rather than a map.
    pub const COUNT: usize = Self::ALL.len();

    /// This slot's place in such an array.
    ///
    /// A search of [`Self::ALL`] rather than a second list written as a match,
    /// which is what makes the result `< COUNT` by construction instead of by
    /// somebody remembering.
    ///
    /// # Panics
    /// A variant that is not in [`Self::ALL`], which is a bug in this file and
    /// not a runtime condition. Const-evaluated wherever the caller is const.
    #[must_use]
    pub const fn index(self) -> usize {
        let mut at = 0;
        // `as u8` and not `==`: a fieldless enum casts in a const fn, and
        // `PartialEq` is not const.
        while at < Self::ALL.len() {
            if Self::ALL[at] as u8 == self as u8 {
                return at;
            }
            at += 1;
        }
        panic!("a BarSlot that is missing from BarSlot::ALL has no place in a per-slot array")
    }
}

/// How long a title spends fading in, on screen, and fading out, in ticks.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TitleTimes {
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
}

impl TitleTimes {
    /// Vanilla's own, which is what a client uses when nothing says otherwise.
    pub const DEFAULT: Self = Self {
        fade_in: 10,
        stay: 70,
        fade_out: 20,
    };
    /// Exactly one second, with no fade at either end.
    ///
    /// For a sequence: a countdown digit that faded out over twenty ticks
    /// would still be on screen when the next one arrived, and the two would
    /// cross-fade into an unreadable smear right at the moment the number
    /// matters.
    pub const TICK: Self = Self {
        fade_in: 0,
        stay: 20,
        fade_out: 0,
    };
}

impl Default for TitleTimes {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Big text across the middle of the screen, and the smaller line under it.
///
/// One value rather than two channels, because the protocol makes the halves
/// separable and the separation is a trap. `ClientboundSetSubtitleTextPacket`
/// does not draw anything: it stores a line, and the *next* title is what puts
/// both on screen. Two independent sends therefore have an order that matters,
/// and a subtitle nobody cleared outlives the title it was written for and
/// reappears under an unrelated one. Carrying them together lets the adapter
/// always write the subtitle first and always write one, empty included, so
/// neither mistake is expressible here.
#[derive(Debug, Clone, PartialEq)]
pub struct Title {
    pub title: Text,
    pub subtitle: Option<Text>,
    pub times: TitleTimes,
}

impl Title {
    /// A title with no line under it, shown for vanilla's own duration.
    #[must_use]
    pub const fn new(title: Text) -> Self {
        Self {
            title,
            subtitle: None,
            times: TitleTimes::DEFAULT,
        }
    }

    /// The smaller line under it.
    #[must_use]
    pub fn under(mut self, subtitle: Text) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    #[must_use]
    pub const fn timed(mut self, times: TitleTimes) -> Self {
        self.times = times;
        self
    }
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

    /// Tell the client whether a mid-air jump is available to it. See
    /// [`Flight`].
    ///
    /// Every call also says the player is not flying, which is what makes a
    /// press a discrete input rather than a state a player can hold: the game
    /// answers a take-off by putting them straight back on this seam's terms,
    /// so the next press is a fresh one.
    fn set_flight(&self, player: PlayerId, flight: Flight);

    /// Push the game's authoritative health onto the client's health bar.
    fn set_health(&self, player: PlayerId, health: f32, max: f32);

    /// Push the game's food bar, in vanilla food points (`0..=20`).
    ///
    /// Separate from [`Server::set_health`] even though vanilla carries the two
    /// in one `SetHealth` packet, because the two change on entirely different
    /// clocks: health moves on every hit, food moves once every seven seconds.
    /// Reassembling the packet is the adapter's problem and not something
    /// fifteen callers should have to hold a spare food level for.
    fn set_food(&self, player: PlayerId, food: u8);

    /// Apply a potion effect to `player`, broadcast to everyone near them --
    /// including the player's own client. That inclusion is the whole point of
    /// a status over a faked impulse: the client owns its movement prediction,
    /// so a slow is *felt* rather than merely seen only once it reaches the
    /// player's own screen. Ended by its own duration, or early with a matching
    /// clear.
    fn status(&self, player: PlayerId, status: Status);

    /// Replace the hotbar wholesale. Called on kit selection and on respawn.
    fn set_hotbar(&self, player: PlayerId, items: &[HotbarItem]);

    fn send_message(&self, player: PlayerId, channel: Channel, text: Text);

    fn broadcast(&self, channel: Channel, text: Text);

    /// Replace a player's sidebar. `lines` is top to bottom.
    fn set_sidebar(&self, player: PlayerId, title: Text, lines: &[SidebarLine]);

    /// Toggle spectator mode: invisible, non-colliding, flying, no attacks.
    fn set_spectating(&self, player: PlayerId, spectating: bool);

    /// Draw a particle effect. Everyone close enough to its origin sees it.
    fn particles(&self, effect: Particles);

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

    /// Move `player`'s experience bar. See [`Experience`].
    fn set_experience(&self, player: PlayerId, experience: Experience);

    /// Put a bar across the top of `player`'s screen, replacing whatever is in
    /// that [`BarSlot`]. There is no way to take one away, because neither
    /// slot has a state with nothing worth putting on it: see
    /// [`crate::module::hud::boss_bar`] and [`crate::module::build_stamp`].
    fn set_boss_bar(&self, player: PlayerId, slot: BarSlot, bar: BossBar);

    /// Put a title across the middle of `player`'s screen.
    fn show_title(&self, player: PlayerId, title: Title);

    /// The same, for everyone.
    fn broadcast_title(&self, title: Title);
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
