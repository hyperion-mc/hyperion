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

pub mod mock;

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

/// A one-shot audiovisual cue. Purely cosmetic, so the game never branches on
/// whether it succeeded.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Cue {
    Explosion,
    Teleport,
    Hurt,
    Death,
    AbilityReady,
    Charge,
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
/// Kept to eight methods on purpose. Anything that can be computed from
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

    fn send_message(&self, player: PlayerId, channel: Channel, text: &str);

    fn broadcast(&self, channel: Channel, text: &str);

    /// Replace a player's sidebar. `lines` is top to bottom.
    fn set_sidebar(&self, player: PlayerId, title: &str, lines: &[String]);

    /// Toggle spectator mode: invisible, non-colliding, flying, no attacks.
    fn set_spectating(&self, player: PlayerId, spectating: bool);

    fn cue(&self, at: Vec3, cue: Cue);
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
