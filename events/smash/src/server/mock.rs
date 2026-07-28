//! A recording [`Server`] so the whole game runs headless in a unit test.
//!
//! Every call is appended to a log. Tests assert on the log rather than on
//! interior state, which keeps them honest about what actually reaches a
//! client: a knockback the game computed but never sent is a bug the log
//! catches and a state assertion does not.

use std::sync::Mutex;

use glam::Vec3;

use super::{Channel, Cue, HotbarItem, PlayerId, Server, SidebarLine, Sound, Text};

/// One thing the game asked the server to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    AddVelocity(PlayerId, Vec3),
    Teleport(PlayerId, Vec3),
    SetHealth(PlayerId, f32, f32),
    SetHotbar(PlayerId, Vec<HotbarItem>),
    Message(PlayerId, Channel, Text),
    Broadcast(Channel, Text),
    Sidebar(PlayerId, Text, Vec<SidebarLine>),
    Spectating(PlayerId, bool),
    Cue(Vec3, Cue),
    /// A positioned sound, heard by everyone in range.
    Sound(Vec3, Sound),
    /// A sound only one player hears.
    SoundTo(PlayerId, Sound),
}

#[derive(Default)]
pub struct MockServer {
    calls: Mutex<Vec<Call>>,
}

impl MockServer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    /// Every call so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// Drain the log. Lets a test assert on one phase without the previous
    /// phase's calls bleeding in.
    pub fn take(&self) -> Vec<Call> {
        core::mem::take(&mut *self.calls.lock().unwrap())
    }

    /// Total velocity the game asked to be applied to `player`. Knockback tests
    /// want the sum, not the individual impulses.
    #[must_use]
    pub fn total_velocity(&self, player: PlayerId) -> Vec3 {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::AddVelocity(id, delta) if *id == player => Some(*delta),
                _ => None,
            })
            .sum()
    }

    /// What was said to `player`, as the words without the styling.
    ///
    /// A test asserting on wording wants the words; a test asserting on colour
    /// reads [`Call::Message`] and looks at the component.
    #[must_use]
    pub fn messages_to(&self, player: PlayerId) -> Vec<String> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Message(id, _, text) if *id == player => Some(text.plain()),
                _ => None,
            })
            .collect()
    }

    /// Every positioned sound so far, with where it played.
    #[must_use]
    pub fn sounds(&self) -> Vec<(Vec3, Sound)> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Sound(at, sound) => Some((*at, *sound)),
                _ => None,
            })
            .collect()
    }

    /// Every sound sent to `player` alone.
    #[must_use]
    pub fn sounds_to(&self, player: PlayerId) -> Vec<Sound> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::SoundTo(id, sound) if *id == player => Some(*sound),
                _ => None,
            })
            .collect()
    }

    #[must_use]
    pub fn broadcasts(&self) -> Vec<String> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Broadcast(_, text) => Some(text.plain()),
                _ => None,
            })
            .collect()
    }
}

impl Server for MockServer {
    fn add_velocity(&self, player: PlayerId, delta: Vec3) {
        self.push(Call::AddVelocity(player, delta));
    }

    fn teleport(&self, player: PlayerId, to: Vec3) {
        self.push(Call::Teleport(player, to));
    }

    fn set_health(&self, player: PlayerId, health: f32, max: f32) {
        self.push(Call::SetHealth(player, health, max));
    }

    fn set_hotbar(&self, player: PlayerId, items: &[HotbarItem]) {
        self.push(Call::SetHotbar(player, items.to_vec()));
    }

    fn send_message(&self, player: PlayerId, channel: Channel, text: Text) {
        self.push(Call::Message(player, channel, text));
    }

    fn broadcast(&self, channel: Channel, text: Text) {
        self.push(Call::Broadcast(channel, text));
    }

    fn set_sidebar(&self, player: PlayerId, title: Text, lines: &[SidebarLine]) {
        self.push(Call::Sidebar(player, title, lines.to_vec()));
    }

    fn set_spectating(&self, player: PlayerId, spectating: bool) {
        self.push(Call::Spectating(player, spectating));
    }

    fn cue(&self, at: Vec3, cue: Cue) {
        self.push(Call::Cue(at, cue));
    }

    fn play_sound(&self, at: Vec3, sound: Sound) {
        self.push(Call::Sound(at, sound));
    }

    fn play_sound_to(&self, player: PlayerId, sound: Sound) {
        self.push(Call::SoundTo(player, sound));
    }
}
