//! Defined the [`Global`] struct which is used to store global data which defines a [`crate::HyperionCore`]
use std::{
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use bevy::prelude::*;
use libdeflater::CompressionLvl;
use valence_protocol::CompressionThreshold;

pub mod config;
pub mod runtime;
pub mod util;

/// Shared data that is shared between the ECS framework and the IO thread.
pub struct Shared {
    /// The compression level to use for the server. This is how long a packet needs to be before it is compressed.
    pub compression_threshold: CompressionThreshold,

    /// The compression level to use for the server. This is the [`libdeflater`] compression level.
    pub compression_level: CompressionLvl,
}

#[derive(Component)]
pub struct Global {
    /// The current tick of the game. This is incremented every 50 ms.
    pub tick: i64,

    /// The maximum amount of time a player is resistant to being hurt. This is weird as this is 20 in vanilla
    /// Minecraft.
    /// However, the check to determine if a player can be hurt actually looks at this value divided by 2
    pub max_hurt_resistant_time: u16,

    /// Data shared between the IO thread and the ECS framework.
    pub shared: Arc<Shared>,

    /// The amount of time from the last packet a player has sent before the server will kick them.
    pub keep_alive_timeout: Duration,

    /// The amount of time the last tick took in milliseconds.
    pub ms_last_tick: f32,

    pub player_count: AtomicUsize,
}

impl Global {
    /// Creates a new [`Global`] with the given shared data.
    #[must_use]
    pub const fn new(shared: Arc<Shared>) -> Self {
        Self {
            tick: 0,
            max_hurt_resistant_time: 20, // actually kinda like 10 vanilla mc is weird
            shared,
            keep_alive_timeout: Duration::from_secs(20),
            ms_last_tick: 0.0,
            player_count: AtomicUsize::new(0),
        }
    }
}
