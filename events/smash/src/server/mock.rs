//! A recording [`Server`] so the whole game runs headless in a unit test.
//!
//! Every call is appended to a log. Tests assert on the log rather than on
//! interior state, which keeps them honest about what actually reaches a
//! client: a knockback the game computed but never sent is a bug the log
//! catches and a state assertion does not.

use std::sync::Mutex;

use glam::Vec3;

use super::{
    BarSlot, BossBar, Channel, Experience, Flight, HotbarItem, Particles, PlayerId, Server,
    SidebarLine, Sound, Status, Text, Title,
};

/// One thing the game asked the server to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    AddVelocity(PlayerId, Vec3),
    Teleport(PlayerId, Vec3),
    /// Whether the client may take a mid-air jump.
    Flight(PlayerId, Flight),
    SetHealth(PlayerId, f32, f32),
    /// A food bar, in vanilla food points.
    SetFood(PlayerId, u8),
    /// A potion effect applied to a player.
    Status(PlayerId, Status),
    SetHotbar(PlayerId, Vec<HotbarItem>),
    Message(PlayerId, Channel, Text),
    Broadcast(Channel, Text),
    Sidebar(PlayerId, Text, Vec<SidebarLine>),
    Spectating(PlayerId, bool),
    Particles(Particles),
    /// A positioned sound, heard by everyone in range.
    Sound(Vec3, Sound),
    /// A sound only one player hears.
    SoundTo(PlayerId, Sound),
    Experience(PlayerId, Experience),
    BossBar(PlayerId, BarSlot, BossBar),
    Title(PlayerId, Title),
    BroadcastTitle(Title),
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

    /// Every flight state pushed to `player`, oldest first.
    ///
    /// The whole series and not the last one, because what the double jump has
    /// to get right is the *sequence*: armed on leaving the ground, disarmed
    /// once the jumps are spent, and one push per change rather than one a
    /// tick. A final-state assertion cannot tell those apart.
    #[must_use]
    pub fn flight_of(&self, player: PlayerId) -> Vec<Flight> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Flight(id, flight) if *id == player => Some(*flight),
                _ => None,
            })
            .collect()
    }

    /// Every status effect applied to `player`, oldest first.
    #[must_use]
    pub fn statuses_of(&self, player: PlayerId) -> Vec<Status> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Status(id, status) if *id == player => Some(*status),
                _ => None,
            })
            .collect()
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
    /// Every particle effect asked for, in order.
    pub fn particles(&self) -> Vec<Particles> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Particles(effect) => Some(effect.clone()),
                _ => None,
            })
            .collect()
    }

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

    /// Every experience bar pushed to `player`, oldest first.
    #[must_use]
    pub fn experience_of(&self, player: PlayerId) -> Vec<Experience> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Experience(id, experience) if *id == player => Some(*experience),
                _ => None,
            })
            .collect()
    }

    /// Every boss bar pushed to `player` in one slot, oldest first.
    ///
    /// Per slot rather than all of them, because the two slots are asserted
    /// about in opposite directions: a test of the match bar wants the run of
    /// values it moved through, and a test of the build stamp wants there to
    /// be exactly one for the life of the process. Merging them would make
    /// each of those quietly depend on the other.
    #[must_use]
    pub fn boss_bars_of(&self, player: PlayerId, slot: BarSlot) -> Vec<BossBar> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::BossBar(id, at, bar) if *id == player && *at == slot => Some(bar.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every title shown to `player` alone.
    #[must_use]
    pub fn titles_to(&self, player: PlayerId) -> Vec<Title> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::Title(id, title) if *id == player => Some(title.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every title shown to everyone.
    #[must_use]
    pub fn broadcast_titles(&self) -> Vec<Title> {
        self.calls()
            .iter()
            .filter_map(|call| match call {
                Call::BroadcastTitle(title) => Some(title.clone()),
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

    fn set_flight(&self, player: PlayerId, flight: Flight) {
        self.push(Call::Flight(player, flight));
    }

    fn set_health(&self, player: PlayerId, health: f32, max: f32) {
        self.push(Call::SetHealth(player, health, max));
    }

    fn set_food(&self, player: PlayerId, food: u8) {
        self.push(Call::SetFood(player, food));
    }

    fn status(&self, player: PlayerId, status: Status) {
        self.push(Call::Status(player, status));
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

    fn particles(&self, effect: Particles) {
        self.push(Call::Particles(effect));
    }

    fn play_sound(&self, at: Vec3, sound: Sound) {
        self.push(Call::Sound(at, sound));
    }

    fn play_sound_to(&self, player: PlayerId, sound: Sound) {
        self.push(Call::SoundTo(player, sound));
    }

    fn set_experience(&self, player: PlayerId, experience: Experience) {
        self.push(Call::Experience(player, experience));
    }

    fn set_boss_bar(&self, player: PlayerId, slot: BarSlot, bar: BossBar) {
        self.push(Call::BossBar(player, slot, bar));
    }

    fn show_title(&self, player: PlayerId, title: Title) {
        self.push(Call::Title(player, title));
    }

    fn broadcast_title(&self, title: Title) {
        self.push(Call::BroadcastTitle(title));
    }
}
