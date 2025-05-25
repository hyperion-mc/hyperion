use bevy::prelude::*;

/// Marks players who are in the handshake state. Player entities will only have 1 state component at a
/// time.
#[derive(Component)]
pub struct Handshake(pub(crate) ());

/// Marks players who are in the status state. Player entities will only have 1 state component at a
/// time.
#[derive(Component)]
pub struct Status(pub(crate) ());

/// Marks players who are in the login state. Player entities will only have 1 state component at a
/// time.
#[derive(Component)]
pub struct Login(pub(crate) ());

/// Marks players who are in the play state. Player entities will only have 1 state component at a
/// time.
#[derive(Component)]
pub struct Play(pub(crate) ());
