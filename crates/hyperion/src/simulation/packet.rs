use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    mem::transmute,
};

use anyhow::Result;
use flecs_ecs::macros::Component;
use hyperion_utils::{Lifetime, LifetimeHandle};
use rustc_hash::FxBuildHasher;
use valence_protocol::{Decode, Packet};

use crate::simulation::handlers::{PacketSwitchQuery, add_builtin_handlers};

type DeserializerFn = for<'packet> fn(
    &HandlerRegistry,
    &'packet [u8],
    &dyn LifetimeHandle<'packet>,
    &mut PacketSwitchQuery<'_>,
) -> Result<()>;
type AnyFn = Box<dyn Send + Sync>;
type Handler<T> = Box<
    dyn for<'packet> Fn(
            &<T as Lifetime>::WithLifetime<'packet>,
            &dyn LifetimeHandle<'packet>,
            &mut PacketSwitchQuery<'_>,
        ) -> Result<()>
        + Send
        + Sync,
>;

fn packet_deserializer<'p, P>(
    registry: &HandlerRegistry,
    mut bytes: &'p [u8],
    handle: &dyn LifetimeHandle<'p>,
    query: &mut PacketSwitchQuery<'_>,
) -> Result<()>
where
    P: Packet + Decode<'static> + Lifetime + 'static,
{
    // If no handler is registered for this packet, skip decoding it
    // TODO: consider moving this check out of the packet deserializer for performance
    if !registry.has_handler::<P>() {
        return Ok(());
    }

    // SAFETY: The transmute to 'static is sound because the result is immediately shortened to
    // the original 'p lifetime. This transmute is necessary due to a technical limitation in the
    // Decode trait; users can only use P with a concrete lifetime, meaning that Decode would
    // decode to that lifetime, but we need to be able to decode to the 'p lifetime
    // TODO: could be unsound if someone implemented Decode<'static> and kept the 'static references in the error
    let packet = P::decode(unsafe { transmute::<&mut &'p [u8], &mut &'static [u8]>(&mut bytes) })?
        .shorten_lifetime::<'p>();

    registry.trigger(&packet, handle, query)?;

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
    pub fn add_handler<'a, P, F>(&mut self, handler: Box<F>)
    where
        P: Lifetime,
        // Needed to allow compiler to infer type of P.
        F: Fn(&P, &dyn LifetimeHandle<'a>, &mut PacketSwitchQuery<'_>) -> Result<()> + Send + Sync,
        // Actual type bounds for Handler<P>
        for<'packet> F: Fn(
                &P::WithLifetime<'packet>,
                &dyn LifetimeHandle<'packet>,
                &mut PacketSwitchQuery<'_>,
            ) -> Result<()>
            + Send
            + Sync
            + 'static,
    {
        // Add the handler to the vector
        self.handlers
            .entry(TypeId::of::<P::WithLifetime<'static>>())
            .or_default()
            .push(unsafe { transmute::<Handler<P>, AnyFn>(handler) });
    }

    // Process a packet, calling all registered handlers
    pub fn process_packet<'p>(
        &self,
        id: i32,
        bytes: &'p [u8],
        handle: &dyn LifetimeHandle<'p>,
        query: &mut PacketSwitchQuery<'_>,
    ) -> Result<()> {
        // Get the deserializer
        let deserializer = self
            .deserializers
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("No deserializer registered for packet ID: {}", id))?;

        deserializer(self, bytes, handle, query)
    }

    #[must_use]
    pub fn has_handler<T>(&self) -> bool
    where
        T: Lifetime,
    {
        self.handlers
            .contains_key(&TypeId::of::<T::WithLifetime<'static>>())
    }

    pub fn trigger<'packet, T>(
        &self,
        value: &T,
        handle: &dyn LifetimeHandle<'packet>,
        query: &mut PacketSwitchQuery<'_>,
    ) -> Result<()>
    where
        T: Lifetime + 'packet,
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
            handler(value.shorten_lifetime_ref(), handle, query)?;
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
            registry.deserializers.insert(PACKET::ID, packet_deserializer::<PACKET<'_>>);
        }
        add_builtin_handlers(&mut registry);
        registry
    }
}
