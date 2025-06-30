//! Components marking a player packet state. Players will have at most 1 state component at a time (they may have no components during state transitions)
//!
//! All players with a state component assigned are guaranteed to have the following components:
/// - [`crate::ConnectionId`]
/// - [`crate::PacketDecoder`]
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
/// - [`crate::simulation::Name`]
/// - [`crate::simulation::Uuid`]
/// - [`crate::simulation::AiTargetable`]
/// - [`crate::simulation::ImmuneStatus`]
/// - [`crate::simulation::ChunkPosition`]
/// - [`crate::egress::sync_chunks::ChunkSendQueue`]
/// - [`crate::simulation::Yaw`]
/// - [`crate::simulation::Pitch`]
/// - [`crate::simulation::skin::PlayerSkin`]
/// - [`crate::simulation::Position`]
#[derive(Component)]
pub struct Play(pub(crate) ());
