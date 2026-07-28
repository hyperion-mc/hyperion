//! Real potion effects: a slow that slows, a speed that speeds.
//!
//! This is the difference between an ability that *looks* like it slows you and
//! one that does. A slow faked with repeated [`motion`](super::motion) impulses
//! reads to the player as lag -- their input and the character disagree -- and
//! it fights the client's own movement prediction rather than joining it. A
//! `minecraft:slowness` effect is applied by the client to its own prediction,
//! so the character moves exactly as slowly as it looks, with no rubber-banding.
//!
//! # Who owns the countdown
//!
//! The client. [`ClientboundUpdateMobEffectPacket`] carries the duration, and
//! the client counts it down and removes the effect itself when it reaches
//! zero. So there is no server-side timer here re-counting the same seconds:
//! that would be two clocks for one deadline, and the CLAUDE.md smell of doing
//! one thing twice. The server's job is two packets -- start it, and, if it has
//! to end before its time, [`clear`] it.
//!
//! An effect that lasts *while a condition holds* -- standing on ice, caught in
//! a web -- is applied with a duration a little longer than the interval it is
//! refreshed at, and [`clear`]ed when the condition ends. Re-applying every
//! tick also works and costs one packet a tick per affected player, which at
//! this game's scale is fine but is not free.
//!
//! [`ClientboundUpdateMobEffectPacket`]: https://minecraft.wiki/w/Java_Edition_protocol

use std::time::Duration;

use flecs_ecs::core::{EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion_minecraft_proto::{
    generated::{packet_id::play::clientbound::PacketId, registry::MobEffect},
    packets::play::clientbound::{RemoveMobEffect, UpdateMobEffect},
};
use hyperion_utils::EntityExt;
use tracing::warn;

use crate::{
    net::{Compose, DataBundle, protocol::Clientbound},
    simulation::Position,
};

/// Ticks per second, the rate the wire counts a duration in.
const TICKS_PER_SECOND: f32 = 20.0;

/// The duration field's value for an effect that does not expire on its own
/// (`ClientboundUpdateMobEffectPacket` treats a negative duration as infinite).
///
/// For an effect that ends on a condition rather than a clock -- immobilised
/// until a tackle animation finishes, slowed until you step off the ice -- so
/// the client never removes it early and [`clear`] is what ends it.
const INFINITE_TICKS: i32 = -1;

/// A potion effect, ready to apply.
///
/// Built from the effect and its amplifier, adjusted by the setters, and ended
/// with [`apply`](Self::apply). Every setter consumes and returns, so the whole
/// thing is one expression:
///
/// ```ignore
/// // Slowness IV on a victim for a second and a half.
/// Status::new(MobEffect::Slowness, 3).seconds(1.5).apply(victim);
/// ```
///
/// The amplifier is zero-based, the way the wire and the game's own code count
/// it: amplifier `0` is the level the tooltip writes as `I`, so `Slowness IV`
/// is amplifier `3`. `Slowness VI` (amplifier `5`) is where a player stops
/// being able to move, which is how an "immobilise" is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct Status {
    effect: MobEffect,
    amplifier: u8,
    duration_ticks: i32,
    ambient: bool,
    particles: bool,
    icon: bool,
}

impl Status {
    /// An effect at the given amplifier, lasting until [`cleared`](clear).
    ///
    /// The default is indefinite because a duration is the thing a caller is
    /// most likely to mean to set and least likely to get right by accident:
    /// an effect with an unset duration that vanished after a tick would be a
    /// silent bug, and one that never vanishes is visible the first time it is
    /// tested.
    pub const fn new(effect: MobEffect, amplifier: u8) -> Self {
        Self {
            effect,
            amplifier,
            duration_ticks: INFINITE_TICKS,
            // A combat effect the player should see on themselves and in their
            // HUD, and that no ability so far wants ambient.
            ambient: false,
            particles: true,
            icon: true,
        }
    }

    /// Last `seconds` and then let the client remove it.
    ///
    /// Rounded to the nearest tick, which is the finest the wire counts, so a
    /// caller asking for 1.58 s gets 32 ticks rather than the 31 a truncation
    /// would drop it to. A duration under half a tick rounds to zero.
    pub fn seconds(mut self, seconds: f32) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the rounded, clamped value is a small tick count well inside i32"
        )]
        let ticks = (seconds * TICKS_PER_SECOND).round().max(0.0) as i32;
        self.duration_ticks = ticks;
        self
    }

    /// Last `duration` and then let the client remove it.
    pub fn duration(self, duration: Duration) -> Self {
        self.seconds(duration.as_secs_f32())
    }

    /// Last a number of ticks. `-1` is [`INFINITE_TICKS`], which does not
    /// expire on its own.
    pub const fn ticks(mut self, ticks: i32) -> Self {
        self.duration_ticks = ticks;
        self
    }

    /// Whether the effect's particles trail the entity. On by default.
    pub const fn particles(mut self, particles: bool) -> Self {
        self.particles = particles;
        self
    }

    /// Whether the effect shows in the player's own HUD. On by default.
    pub const fn icon(mut self, icon: bool) -> Self {
        self.icon = icon;
        self
    }

    /// Whether the effect is "ambient", the fainter presentation a beacon
    /// gives. Off by default; a combat effect is not ambient.
    pub const fn ambient(mut self, ambient: bool) -> Self {
        self.ambient = ambient;
        self
    }

    /// The flag byte, exactly as `ClientboundUpdateMobEffectPacket` packs it.
    ///
    /// `FLAG_AMBIENT = 1`, `FLAG_VISIBLE = 2`, `FLAG_SHOW_ICON = 4`. The fourth
    /// vanilla flag, `FLAG_BLEND = 8`, is the darkness-effect sky blend and is
    /// not something an ability sets, so it is never written here.
    #[must_use]
    pub const fn flags(&self) -> i8 {
        let mut flags = 0;
        if self.ambient {
            flags |= 1;
        }
        if self.particles {
            flags |= 2;
        }
        if self.icon {
            flags |= 4;
        }
        flags
    }

    /// The packet that starts this effect on `entity`.
    ///
    /// Public so a wire test can assert on the bytes without a running server,
    /// which is the only honest way to check that the amplifier, duration and
    /// flags a caller set are the ones that leave.
    #[must_use]
    pub fn packet(&self, entity_id: i32) -> UpdateMobEffect {
        UpdateMobEffect {
            entity_id,
            effect: self.effect.id(),
            // The wire amplifier is a varint, and `u8` is the range an ability
            // uses; widening here cannot lose anything.
            effect_amplifier: i32::from(self.amplifier),
            effect_duration_ticks: self.duration_ticks,
            flags: self.flags(),
        }
    }

    /// Apply this effect to `entity`.
    ///
    /// Broadcast to the clients near the entity, which for a player includes
    /// their own -- the client that has to apply the movement modifier for the
    /// effect to be felt rather than only seen.
    pub fn apply(self, entity: EntityView<'_>) {
        let id = entity.minecraft_id();
        let chunk = entity.try_get::<&Position>(Position::to_chunk);
        let Some(chunk) = chunk else {
            warn!("cannot apply a status effect to an entity with no position");
            return;
        };
        entity.world().get::<&Compose>(|compose| {
            let mut bundle = DataBundle::new(compose);
            let packet = self.packet(id);
            if let Err(error) = bundle.add_packet(Clientbound::new(
                PacketId::UpdateMobEffect.to_raw(),
                &packet,
            )) {
                warn!("dropping a status effect: {error}");
                return;
            }
            if let Err(error) = bundle.broadcast_local(chunk) {
                warn!("dropping a status effect: {error}");
            }
        });
    }
}

/// End an effect on `entity` before its duration is up.
///
/// For the effects that end on a condition rather than a clock: stepping off
/// the ice, breaking free of a web, the tackle animation finishing. An effect
/// that was applied with a real duration does not need this -- the client
/// removes it on its own -- and clearing one that is not present is a harmless
/// no-op the client ignores.
pub fn clear(entity: EntityView<'_>, effect: MobEffect) {
    let id = entity.minecraft_id();
    let chunk = entity.try_get::<&Position>(Position::to_chunk);
    let Some(chunk) = chunk else {
        warn!("cannot clear a status effect from an entity with no position");
        return;
    };
    entity.world().get::<&Compose>(|compose| {
        let mut bundle = DataBundle::new(compose);
        let packet = RemoveMobEffect {
            entity_id: id,
            effect: effect.id(),
        };
        if let Err(error) = bundle.add_packet(Clientbound::new(
            PacketId::RemoveMobEffect.to_raw(),
            &packet,
        )) {
            warn!("dropping a status clear: {error}");
            return;
        }
        if let Err(error) = bundle.broadcast_local(chunk) {
            warn!("dropping a status clear: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use flecs_ecs::prelude::*;
    use hyperion_proxy_proto::ArchivedServerToProxyMessage;

    use super::*;
    use crate::{
        HyperionCore,
        simulation::{Player, Velocity, entity_kind::EntityKind},
    };

    /// A slow reaches the victim's own client.
    ///
    /// The whole reason a status beats a faked impulse is that the client owns
    /// its movement prediction, so the effect has to arrive at the *victim's*
    /// screen to be felt rather than only seen by bystanders. A broadcast that
    /// excluded the victim -- the way a game often excludes the actor of an
    /// event to avoid echoing it back -- would slow everyone's view of them and
    /// leave the one player who needs it moving at full speed.
    ///
    /// So this drives [`Status::apply`] against a captured proxy channel and
    /// asserts the effect leaves as a *local* broadcast around the victim that
    /// excludes nobody (`exclude == 0`, the sentinel for [`None`]), which is
    /// exactly the client that owns the prediction being included.
    #[test]
    fn a_slow_broadcasts_to_the_victims_own_client() {
        let world = World::new();
        world.import::<HyperionCore>();

        let victim = world
            .entity()
            .add_enum(EntityKind::Player)
            .add(Player::id())
            .set(Position::new(1.5, 64.0, -2.5))
            .set(Velocity::default())
            .id();

        // Swap in a Compose whose proxy channels we hold, so what a broadcast
        // would send is readable here rather than lost to a socket. Done after
        // the entity exists, so only the slow -- not the spawn -- lands on it.
        let (compose, mut near, _far) = crate::net::tests::two_proxies();
        world.set(compose);

        Status::new(MobEffect::Slowness, 5)
            .seconds(5.0)
            .apply(world.entity_from_id(victim));

        let bytes = near
            .try_recv()
            .expect("apply broadcast nothing to the victim's proxy");
        // `encode_proxy_message` writes an eight-byte big-endian length before
        // the rkyv body; the body is what the archived message reads through.
        let body = &bytes[size_of::<u64>()..];
        let message = unsafe { rkyv::access_unchecked::<ArchivedServerToProxyMessage<'_>>(body) };
        match message {
            ArchivedServerToProxyMessage::BroadcastLocal(local) => {
                assert_eq!(
                    local.exclude.to_native(),
                    0,
                    "a slow that excludes the victim never reaches the client that has to feel it"
                );
            }
            _ => panic!("a slow must leave as a local broadcast around the victim"),
        }
        assert!(
            near.try_recv().is_err(),
            "one apply must be one packet, not several"
        );

        // Drop the world while the proxy receivers are still alive: tearing it
        // down fires an `OnRemove` broadcast for the victim, and a receiver
        // already gone would turn that teardown into a `SendError` panic.
        drop(world);
    }
}
