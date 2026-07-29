//! A static, protocol-frozen day time for the arena sky.
//!
//! 26.2 drives the sun from a per-world clock the client advances itself: the
//! server sends the clock's `rate` once and the client interpolates from there
//! (see [`hyperion_minecraft_proto::packets::play_login::SetTime`] and
//! `ServerClockManager`). hyperion never ticks that clock, so with no `SetTime`
//! the client free-runs its own daylight cycle and the sun drifts. Sending the
//! overworld clock once with `rate` `0.0` freezes it at [`WorldTime::day_time`]
//! without the server resending time every tick.
//!
//! This is one singleton both events share: bedwars and smash both want a fixed
//! arena sky. The default is noon; an event overrides it before players join
//! with `world.set(WorldTime { day_time: ... })`. The wire freeze is the whole
//! mechanism -- the `advance_time`/`doDaylightCycle` gamerule is only the
//! server-side toggle that makes vanilla serialise the same `rate` `0.0`, and
//! hyperion has no gamerule state to mirror.

use flecs_ecs::prelude::*;

/// Ticks in a full Minecraft day; day time wraps at this.
pub const TICKS_PER_DAY: i64 = 24_000;

/// Day time with the sun at its zenith. The default frozen sky.
pub const NOON: i64 = 6_000;

/// The frozen day time for the overworld sky, in ticks `[0, 24000)`.
///
/// A singleton read on join to build the freeze
/// [`SetTime`](hyperion_minecraft_proto::packets::play_login::SetTime).
/// Override it per event with `world.set(WorldTime { day_time })`.
#[derive(Component, Debug, Clone, Copy)]
pub struct WorldTime {
    /// Day time positioning the sun: `6000` noon, `18000` midnight, `0`/`24000`
    /// sunrise. Values outside `[0, 24000)` are wrapped by the client.
    pub day_time: i64,
}

impl Default for WorldTime {
    fn default() -> Self {
        Self { day_time: NOON }
    }
}

/// Registers [`WorldTime`] as a singleton and installs its noon default so the
/// join path always finds one to freeze the sky at. Events that want a
/// different sky call `world.set(WorldTime { .. })` after this is imported.
///
/// The `add_trait::<flecs::Singleton>()` is load-bearing, not decoration: a
/// bare `world.set` stores the value but never registers the component, so
/// `world.get::<&WorldTime>` asserts "component is not registered" the first
/// time a player joins. A release build compiles that assert out and reads the
/// value anyway, which is why the miss only surfaced when the smash server was
/// booted in the dev profile. `HyperionCore` imports this directly, next to
/// `SpatialModule`, so every event gets it regardless of what else it imports.
#[derive(Component)]
pub struct WorldTimeModule;

impl Module for WorldTimeModule {
    fn module(world: &World) {
        world
            .component::<WorldTime>()
            .add_trait::<flecs::Singleton>();
        world.set(WorldTime::default());
    }
}
