use bevy::prelude::*;
use derive_more::Deref;
use hyperion_packet_macros::for_each_state;

use crate::net::ConnectionId;

#[derive(Copy, Clone, Debug, Deref, Event)]
pub struct Packet<T> {
    sender: Entity,
    connection_id: ConnectionId,

    #[deref]
    body: T,
}

impl<T> Packet<T> {
    pub fn new(sender: Entity, connection_id: ConnectionId, body: T) -> Self {
        Self {
            sender,
            connection_id,
            body,
        }
    }

    /// Entity of the player who sent this packet
    pub fn sender(&self) -> Entity {
        self.sender
    }

    /// Connection ID of the player who sent this packet. This is included for convenience; it is
    /// the same connection ID component in the [`Packet::sender`] entity.
    pub fn connection_id(&self) -> ConnectionId {
        self.connection_id
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
