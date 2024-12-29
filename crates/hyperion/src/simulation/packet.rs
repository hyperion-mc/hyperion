use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    mem::transmute,
};

use anyhow::Result;
use flecs_ecs::macros::Component;
use rustc_hash::FxBuildHasher;
use valence_protocol::{Decode, Packet};

use crate::{common::util::Lifetime, simulation::handlers::PacketSwitchQuery};

type DeserializerFn = fn(&HandlerRegistry, &[u8], &mut PacketSwitchQuery<'_>) -> Result<()>;
type AnyHandler = unsafe fn();
type Handler<T> = fn(&<T as Lifetime>::WithLifetime<'_>, &mut PacketSwitchQuery<'_>) -> Result<()>;

fn packet_deserializer<'p, P>(
    registry: &HandlerRegistry,
    mut bytes: &'p [u8],
    query: &mut PacketSwitchQuery<'_>,
) -> Result<()>
where
    P: Packet + Decode<'static> + Lifetime + 'static,
{
    // SAFETY: The transmute to 'static is sound because the result is immediately shortened to
    // the original 'p lifetime. This transmute is necessary due to a technical limitation in the
    // Decode trait; users can only use P with a concrete lifetime, meaning that Decode would
    // decode to that lifetime, but we need to be able to decode to the 'p lifetime
    // TODO: could be unsound if someone implemented Decode<'static> and kept the 'static references in the error
    let packet = P::decode(unsafe { transmute::<&mut &'p [u8], &mut &'static [u8]>(&mut bytes) })?
        .shorten_lifetime::<'p>();

    registry.trigger(packet, query)?;

    Ok(())
}

#[derive(Component, Default)]
pub struct HandlerRegistry {
    // Store deserializer and multiple handlers separately
    deserializers: HashMap<i32, DeserializerFn, FxBuildHasher>,
    handlers: HashMap<TypeId, Vec<AnyHandler>, FxBuildHasher>,
}

impl HandlerRegistry {
    // Register a packet type's deserializer
    pub fn register_packet<P>(&mut self)
    where
        P: Packet + Decode<'static> + Lifetime + 'static,
    {
        self.deserializers.insert(P::ID, packet_deserializer::<P>);
    }

    // Add a handler
    pub fn add_handler<P>(&mut self, handler: Handler<P>)
    where
        P: Lifetime + 'static,
    {
        // Add the handler to the vector
        self.handlers
            .entry(TypeId::of::<P::WithLifetime<'static>>())
            .or_default()
            .push(unsafe { transmute::<Handler<P>, AnyHandler>(handler) });
    }

    // Process a packet, calling all registered handlers
    pub fn process_packet(
        &self,
        id: i32,
        bytes: &[u8],
        query: &mut PacketSwitchQuery<'_>,
    ) -> Result<()> {
        // Get the deserializer
        let deserializer = self
            .deserializers
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("No deserializer registered for packet ID: {}", id))?;

        deserializer(self, bytes, query)
    }

    pub fn trigger<T>(&self, value: T, query: &mut PacketSwitchQuery<'_>) -> Result<()>
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
            // to make any assumptions about the length of the lifetime of the T passed to the
            // handler function.
            let handler = unsafe { &*std::ptr::from_ref(&handler).cast::<Handler<T>>() };

            // The existing lifetime of T is okay, but shorten_lifetime is needed because the type
            // checker expects a value with type T::WithLifetime.
            handler((&value).shorten_lifetime(), query)?;
        }

        Ok(())
    }
}
