//! Components marking a player packet state. Players will have at most 1 state component at a time (they may have no components during state transitions)
//!
//! All players with a state component assigned are guaranteed to have the following components:
/// - [`hyperion::ConnectionId`]
/// - [`hyperion::PacketDecoder`]
use bevy::prelude::*;

/// Marks players who are in the handshake state.
#[derive(Component)]
pub struct Handshake(pub(crate) ());

/// Marks players who are in the status state.
#[derive(Component)]
pub struct Status(pub(crate) ());

/// Marks players who are in the login state.
#[derive(Component)]
pub struct Login(pub(crate) ());

/// Marks players who are in the play state.
///
/// Players in this state are guaranteed to have the following components:
/// - [`hyperion::simulation::Name`]
/// - [`hyperion::simulation::Uuid`]
/// - [`hyperion::simulation::AiTargetable`]
/// - [`hyperion::simulation::ImmuneStatus`]
/// - [`hyperion::simulation::ChunkPosition`]
/// - [`hyperion::simulation::ChunkSendQueue`]
/// - [`hyperion::simulation::Yaw`]
/// - [`hyperion::simulation::Pitch`]
///
/// They may, but are not required to, have the following components:
/// - [`hyperion::simulation::skin::PlayerSkin`]: is added once the player skin loads
/// - [`hyperion::simulation::Position`]: is not set by Hyperion - the event code must add this
/// component
#[derive(Component)]
pub struct Play(pub(crate) ());
