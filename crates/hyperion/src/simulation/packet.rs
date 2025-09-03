use std::cmp::Ordering;

use bevy::prelude::*;
use derive_more::{Deref, From, Into};
use hyperion_packet_macros::for_each_state;
use hyperion_utils::EntityExt;

use crate::net::ConnectionId;

#[derive(Copy, Clone, Debug, Deref, Event)]
pub struct Packet<T> {
    sender: Entity,
    connection_id: ConnectionId,
    #[allow(clippy::struct_field_names)]
    packet_id: u64,

    #[deref]
    body: T,
}

impl<T> Packet<T> {
    pub const fn new(sender: Entity, connection_id: ConnectionId, packet_id: u64, body: T) -> Self {
        Self {
            sender,
            connection_id,
            packet_id,
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

    /// Unique monotonically-increasing packet id
    pub const fn packet_id(&self) -> u64 {
        self.packet_id
    }
}

/// Packet ordered by the time it was received by the server
#[derive(Copy, Clone, Debug, Deref, From, Into)]
pub struct OrderedPacketRef<'a, T>(&'a Packet<T>);

impl<T, U> PartialOrd<OrderedPacketRef<'_, U>> for OrderedPacketRef<'_, T> {
    fn partial_cmp(&self, other: &OrderedPacketRef<'_, U>) -> Option<Ordering> {
        self.packet_id().partial_cmp(&other.packet_id())
    }
}

impl<T, U> PartialEq<OrderedPacketRef<'_, U>> for OrderedPacketRef<'_, T> {
    fn eq(&self, other: &OrderedPacketRef<'_, U>) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

for_each_state! {
    #{
        pub mod #state {
            #for_each_packet! {
                #{
                    pub type #packet_name = super::Packet<#static_valence_packet>;
                }
            }
        }
    }
}

pub struct PacketPlugin;

impl Plugin for PacketPlugin {
    fn build(&self, app: &mut App) {
        for_each_state! {
            #{
                #for_each_packet! {
                    #{
                        app.add_event::<#state::#packet_name>();
                    }
                }
            }
        }
    }
}
