use std::{collections::HashMap, ptr::NonNull};

use anyhow::Result;
use flecs_ecs::core::EntityView;
use rustc_hash::FxBuildHasher;
use valence_protocol::{Decode, Packet};

use crate::simulation::handlers::PacketSwitchQuery;

// We'll store the deserialization function separately from handlers
type DeserializerFn =
    Box<dyn Fn(&HandlerRegistry, &[u8], &mut PacketSwitchQuery<'_>) -> Result<()>>;

type PacketHandler = Box<dyn Fn(NonNull<u8>, &mut PacketSwitchQuery<'_>) -> Result<()>>;

#[derive(Default)]
pub struct HandlerRegistry {
    // Store deserializer and multiple handlers separately
    deserializers: HashMap<i32, DeserializerFn, FxBuildHasher>,
    handlers: HashMap<i32, Vec<PacketHandler>, FxBuildHasher>,
}

impl HandlerRegistry {
    // Register a packet type's deserializer
    pub fn register_packet<'p, P>(&mut self)
    where
        P: Packet + Send + Sync + Decode<'p>,
    {
        let deserializer: DeserializerFn = Box::new(
            |registry: &Self, bytes: &[u8], query: &mut PacketSwitchQuery<'_>| -> Result<()> {
                // transmute to bypass lifetime issue with Decode
                // packet is dropped at end of scope and references in handlers are locked to the scope of the handler
                let mut bytes = unsafe { std::mem::transmute::<&[u8], &'static [u8]>(bytes) };
                let mut packet = P::decode(&mut bytes)?;
                // packet is always non-null, swap to NonNull::from_mut after stabilization
                let ptr = unsafe { NonNull::new_unchecked(&mut packet).cast::<u8>() };

                // Get all handlers for this packet type
                let handlers = registry.handlers.get(&P::ID).ok_or_else(|| {
                    anyhow::anyhow!("No handlers registered for packet ID: {}", P::ID)
                })?;

                // Call all handlers with the deserialized packet
                for handler in handlers {
                    handler(ptr, query)?;
                }

                Ok(())
            },
        );

        self.deserializers.insert(P::ID, deserializer);
        // Initialize the handlers vector if it doesn't exist
        self.handlers.entry(P::ID).or_default();
    }

    // Add a handler for a packet type
    pub fn add_handler<'p, P>(
        &mut self,
        handler: impl for<'a> Fn(&'a P, &mut PacketSwitchQuery<'_>) -> Result<()>
        + Send
        + Sync
        + 'static,
    ) where
        P: Packet + Send + Sync + Decode<'p>,
    {
        // Ensure the packet type is registered
        if !self.deserializers.contains_key(&P::ID) {
            self.register_packet::<P>();
        }

        // Wrap the typed handler to work with Any
        let boxed_handler: PacketHandler = Box::new(move |any_packet, entity| {
            let packet = unsafe { any_packet.cast::<P>().as_ref() };
            handler(packet, entity)
        });

        // Add the handler to the vector
        self.handlers.entry(P::ID).or_default().push(boxed_handler);
    }

    // Process a packet, calling all registered handlers
    pub fn process_packet(&self, id: i32, bytes: &[u8], query: &mut PacketSwitchQuery<'_>) -> Result<()> {
        // Get the deserializer
        let deserializer = self
            .deserializers
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("No deserializer registered for packet ID: {}", id))?;

        deserializer(self, bytes, query)
    }
}
