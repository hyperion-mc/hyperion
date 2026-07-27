//! The event handler registry, and the serverbound play bodies the proto
//! crate's generator could not describe.
//!
//! Packet dispatch itself lives in [`crate::simulation::handlers`], which
//! matches on the generated `PacketId` for protocol 776 and calls a handler
//! directly. What is left here is the type-keyed table other crates register
//! into for the *events* a handler raises: `InteractEvent`,
//! `CommandCompletionRequest` and `ClientStatusEvent` all reach their
//! subscribers through [`HandlerRegistry::trigger`].

use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    mem::transmute,
};

use anyhow::Result;
use flecs_ecs::macros::Component;
use hyperion_utils::Lifetime;
use rustc_hash::FxBuildHasher;

use crate::simulation::handlers::PacketSwitchQuery;

type AnyFn = Box<dyn Send + Sync>;
type Handler<T> = Box<
    dyn for<'packet> Fn(
            &<T as Lifetime>::WithLifetime<'packet>,
            &mut PacketSwitchQuery<'_>,
        ) -> Result<()>
        + Send
        + Sync,
>;

/// Subscribers to the events raised while a packet is handled, keyed by event
/// type.
#[derive(Component, Default)]
pub struct HandlerRegistry {
    handlers: HashMap<TypeId, Vec<AnyFn>, FxBuildHasher>,
}

impl HandlerRegistry {
    // TODO: With this current system, closures infer that 'a is a specific lifetime if the type isn't specified. Unsure if there's a way to fix it while allowing P to be inferred.
    pub fn add_handler<P, F>(&mut self, handler: Box<F>)
    where
        P: Lifetime,
        // Needed to allow compiler to infer type of P.
        F: Fn(&P, &mut PacketSwitchQuery<'_>) -> Result<()> + Send + Sync,
        // Actual type bounds for Handler<P>
        for<'packet> F: Fn(&P::WithLifetime<'packet>, &mut PacketSwitchQuery<'_>) -> Result<()>
            + Send
            + Sync
            + 'static,
    {
        // Add the handler to the vector
        self.handlers
            .entry(TypeId::of::<P::WithLifetime<'static>>())
            .or_default()
            // SAFETY: Handler<P> and Box<dyn Send + Sync> are both thin boxed trait objects of the
            // same size, and the value is only ever read back through the matching Handler<P> type
            // in HandlerRegistry::trigger.
            .push(unsafe { transmute::<Handler<P>, AnyFn>(handler) });
    }

    #[must_use]
    pub fn has_handler<T>(&self) -> bool
    where
        T: Lifetime,
    {
        self.handlers
            .contains_key(&TypeId::of::<T::WithLifetime<'static>>())
    }

    pub fn trigger<T>(&self, value: &T, query: &mut PacketSwitchQuery<'_>) -> Result<()>
    where
        T: Lifetime,
    {
        // Get all handlers for this type
        let handlers = self
            .handlers
            .get(&TypeId::of::<T::WithLifetime<'static>>())
            .ok_or_else(|| {
                anyhow::anyhow!("No handlers registered for type {}", type_name::<T>())
            })?;

        // Call all handlers
        for handler in handlers {
            // SAFETY: The underlying handler type is Handler<T> because the type of T matches the
            // type of the value passed to trigger, disregarding lifetimes. It is sound to pass a T
            // of any lifetime to the handler because the borrow checker doesn't allow the handler
            // to make any assumptions about the length of the lifetime of the T
            let handler = unsafe { &*std::ptr::from_ref(handler).cast::<Handler<T>>() };

            // shorten_lifetime is only needed because the handler accepts T::WithLifetime
            handler(value.shorten_lifetime_ref(), query)?;
        }

        Ok(())
    }
}

/// Serverbound play bodies `protocol.json` does not describe in full.
///
/// `hyperion-minecraft-proto`'s `build.rs` refuses to generate a layout with an
/// unresolved leaf anywhere in it, so a handful of packets a playing client
/// sends every session have no generated struct. Each type here names the leaf
/// that stopped the generator and the codec it was written against instead, so
/// a reader can check it against the same decompiled source.
///
/// Nothing about these is hyperion-specific; they belong in the proto crate as
/// soon as it grows a hand-written serverbound module.
pub mod serverbound {
    use hyperion_minecraft_proto::{Decode, Reader, Result};

    /// `MessageSignature.BYTES`: an Ed25519 signature over the chat message.
    pub const MESSAGE_SIGNATURE_BYTES: usize = 256;

    /// `LastSeenMessagesTracker.window` is 20 wide, so the acknowledgement
    /// bitset is a fixed three bytes rather than a length-prefixed one.
    pub const LAST_SEEN_ACKNOWLEDGED_BYTES: usize = 3;

    /// The tail every signed chat packet carries: which of the messages the
    /// client has seen it is acknowledging.
    ///
    /// Layout from `LastSeenMessages$Update#STREAM_CODEC`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct LastSeenMessages {
        /// How far the acknowledged window has advanced.
        pub offset: i32,
        /// One bit per entry in the client's 20-message window.
        pub acknowledged: [u8; LAST_SEEN_ACKNOWLEDGED_BYTES],
        /// Checksum over the acknowledged set, so the server can tell a
        /// desynchronised window from an empty one.
        pub checksum: i8,
    }

    impl Decode<'_> for LastSeenMessages {
        fn decode(reader: &mut Reader<'_>) -> Result<Self> {
            let offset = reader.var_int()?;
            // `take` returns exactly this many bytes or errors, so the copy
            // cannot be short.
            let mut acknowledged = [0_u8; LAST_SEEN_ACKNOWLEDGED_BYTES];
            acknowledged.copy_from_slice(reader.take(LAST_SEEN_ACKNOWLEDGED_BYTES)?);
            let checksum = reader.i8()?;

            Ok(Self {
                offset,
                acknowledged,
                checksum,
            })
        }
    }

    /// `minecraft:chat`, sent serverbound as play id 9.
    ///
    /// Layout from
    /// `net.minecraft.network.protocol.game.ServerboundChatPacket#STREAM_CODEC`.
    /// The generator skipped it because the signature is written by
    /// `output.writeBytes(signature.bytes)`, a raw fixed-width write with no
    /// codec to name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Chat<'a> {
        /// What the player typed. `ServerboundChatPacket.MAX_MESSAGE_LENGTH`.
        pub message: &'a str,
        /// Client clock at send time, in epoch milliseconds. The server rejects
        /// a signature whose timestamp is too far from its own clock.
        pub time_stamp: i64,
        /// Salt mixed into the signature.
        pub salt: i64,
        /// Exactly [`MESSAGE_SIGNATURE_BYTES`] when the client has a chat
        /// session; absent when chat signing is off.
        pub signature: Option<&'a [u8]>,
        /// Which previously seen messages this one is chained to.
        pub last_seen: LastSeenMessages,
    }

    impl Chat<'_> {
        /// `ServerboundChatPacket.MAX_MESSAGE_LENGTH`.
        pub const MAX_MESSAGE_LENGTH: usize = 256;
    }

    impl<'a> Decode<'a> for Chat<'a> {
        fn decode(reader: &mut Reader<'a>) -> Result<Self> {
            let message = reader.string_with_limit(Self::MAX_MESSAGE_LENGTH)?;
            let time_stamp = reader.i64()?;
            let salt = reader.i64()?;
            let signature = reader
                .bool()?
                .then(|| reader.take(MESSAGE_SIGNATURE_BYTES))
                .transpose()?;
            let last_seen = LastSeenMessages::decode(reader)?;

            Ok(Self {
                message,
                time_stamp,
                salt,
                signature,
                last_seen,
            })
        }
    }

    /// `minecraft:player_input`, sent serverbound as play id 43.
    ///
    /// Layout from `net.minecraft.world.entity.player.Input#STREAM_CODEC`,
    /// which the generator skipped because the flags are folded into one byte
    /// by a statement per field rather than by a codec.
    ///
    /// This is where sneaking lives in 26.2. `ServerboundPlayerCommandPacket`
    /// used to carry `PRESS_SHIFT_KEY`/`RELEASE_SHIFT_KEY` and no longer does,
    /// so a server that only reads player commands never sees a player crouch.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PlayerInput(pub u8);

    impl PlayerInput {
        const FORWARD: u8 = 1;
        const BACKWARD: u8 = 2;
        const LEFT: u8 = 4;
        const RIGHT: u8 = 8;
        const JUMP: u8 = 16;
        const SHIFT: u8 = 32;
        const SPRINT: u8 = 64;

        /// Whether the forward key is held.
        #[must_use]
        pub const fn forward(self) -> bool {
            self.0 & Self::FORWARD != 0
        }

        /// Whether the back key is held.
        #[must_use]
        pub const fn backward(self) -> bool {
            self.0 & Self::BACKWARD != 0
        }

        /// Whether the left strafe key is held.
        #[must_use]
        pub const fn left(self) -> bool {
            self.0 & Self::LEFT != 0
        }

        /// Whether the right strafe key is held.
        #[must_use]
        pub const fn right(self) -> bool {
            self.0 & Self::RIGHT != 0
        }

        /// Whether the jump key is held.
        #[must_use]
        pub const fn jump(self) -> bool {
            self.0 & Self::JUMP != 0
        }

        /// Whether the sneak key is held.
        #[must_use]
        pub const fn shift(self) -> bool {
            self.0 & Self::SHIFT != 0
        }

        /// Whether the sprint key is held.
        #[must_use]
        pub const fn sprint(self) -> bool {
            self.0 & Self::SPRINT != 0
        }
    }

    impl Decode<'_> for PlayerInput {
        fn decode(reader: &mut Reader<'_>) -> Result<Self> {
            Ok(Self(reader.u8()?))
        }
    }

    /// `minecraft:player_abilities`, sent serverbound as play id 40.
    ///
    /// Layout from
    /// `net.minecraft.network.protocol.game.ServerboundPlayerAbilitiesPacket#STREAM_CODEC`,
    /// which the generator skipped because the byte is assembled by a branch
    /// (`if (this.isFlying) { bitfield = ... }`).
    ///
    /// The client only ever reports the flying bit; the other bits of the
    /// clientbound abilities byte are server-owned and echoed back as zero.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PlayerAbilities(pub u8);

    impl PlayerAbilities {
        const FLYING: u8 = 2;

        /// Whether the client is asking to fly.
        #[must_use]
        pub const fn is_flying(self) -> bool {
            self.0 & Self::FLYING != 0
        }
    }

    impl Decode<'_> for PlayerAbilities {
        fn decode(reader: &mut Reader<'_>) -> Result<Self> {
            Ok(Self(reader.u8()?))
        }
    }
}
