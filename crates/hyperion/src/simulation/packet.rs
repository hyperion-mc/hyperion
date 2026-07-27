use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    mem::transmute,
};

use anyhow::Result;
use derive_more::Deref;
use flecs_ecs::{core::Entity, macros::Component};
use hyperion_utils::{EntityExt, Lifetime};
use rustc_hash::FxBuildHasher;
use valence_protocol::{DecodeBytes, Packet as PacketTrait};

use crate::{
    net::{ConnectionId, decoder::BorrowedPacketFrame},
    simulation::handlers::{PacketSwitchQuery, add_builtin_handlers},
};

/// A packet which has been decoded, tagged with the player who sent it.
#[derive(Copy, Clone, Debug, Deref)]
pub struct Packet<T> {
    sender: Entity,
    connection_id: ConnectionId,

    #[deref]
    body: T,
}

impl<T> Packet<T> {
    pub const fn new(sender: Entity, connection_id: ConnectionId, body: T) -> Self {
        Self {
            sender,
            connection_id,
            body,
        }
    }

    /// Entity of the player who sent this packet
    pub const fn sender(&self) -> Entity {
        self.sender
    }

    /// Connection id of the player who sent this packet. This is included for convenience; it is
    /// the same connection id component in the [`Packet::sender`] entity.
    pub const fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    /// Minecraft id of the player who sent this packet. This is included for convenience; it is
    /// the same Minecraft id in the [`Packet::sender`] entity.
    pub fn minecraft_id(&self) -> i32 {
        self.sender().minecraft_id()
    }
}

/// One alias per play packet, naming the decoded body a handler receives.
///
/// Only play: the states before it are handled by [`crate::net::protocol`],
/// which decodes straight into the proto crate's types rather than through a
/// registry.
pub mod play {
    hyperion_packet_macros::for_each_play_c2s_packet! {
        #{
            pub type #packet_name = super::Packet<#static_valence_packet>;
        }
    }
}

type DeserializerFn =
    fn(&HandlerRegistry, BorrowedPacketFrame, &mut PacketSwitchQuery<'_>) -> Result<()>;
type AnyFn = Box<dyn Send + Sync>;
type Handler<T> = Box<
    dyn for<'packet> Fn(
            &<T as Lifetime>::WithLifetime<'packet>,
            &mut PacketSwitchQuery<'_>,
        ) -> Result<()>
        + Send
        + Sync,
>;

fn packet_deserializer<P>(
    registry: &HandlerRegistry,
    frame: BorrowedPacketFrame,
    query: &mut PacketSwitchQuery<'_>,
) -> Result<()>
where
    P: PacketTrait + DecodeBytes + Lifetime + 'static,
{
    // If no handler is registered for this packet, skip decoding it
    // TODO: consider moving this check out of the packet deserializer for performance
    if !registry.has_handler::<P>() {
        return Ok(());
    }

    let packet = frame.decode::<P>()?;

    registry.trigger(&packet, query)?;

    Ok(())
}

#[derive(Component)]
pub struct HandlerRegistry {
    // Store deserializer and multiple handlers separately
    deserializers: HashMap<i32, DeserializerFn, FxBuildHasher>,
    handlers: HashMap<TypeId, Vec<AnyFn>, FxBuildHasher>,
}

impl HandlerRegistry {
    // Add a handler
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

    // Process a packet, calling all registered handlers
    pub fn process_packet(
        &self,
        frame: BorrowedPacketFrame,
        query: &mut PacketSwitchQuery<'_>,
    ) -> Result<()> {
        let id = frame.id;

        // Get the deserializer
        let deserializer = self
            .deserializers
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("No deserializer registered for packet ID: {id}"))?;

        deserializer(self, frame, query)
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

impl Default for HandlerRegistry {
    fn default() -> Self {
        let mut registry = Self {
            deserializers: HashMap::default(),
            handlers: HashMap::default(),
        };
        hyperion_packet_macros::for_each_static_play_c2s_packet! {
            registry.deserializers.insert(PACKET::ID, packet_deserializer::<PACKET>);
        }
        hyperion_packet_macros::for_each_lifetime_play_c2s_packet! {
            registry.deserializers.insert(PACKET::ID, packet_deserializer::<PACKET<'static>>);
        }
        add_builtin_handlers(&mut registry);
        registry
    }
}
