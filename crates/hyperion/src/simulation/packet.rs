use bevy::prelude::*;
use hyperion_packet_macros::for_each_state;

use crate::net::ConnectionId;

#[derive(Debug, Event)]
pub struct Packet<T> {
    /// Entity of the player who sent this packet
    pub sender: Entity,

    /// Connection ID of the player who sent this packet. This is included for convenience; it is
    /// the same connection ID component in the [`Packet::sender`] entity.
    pub connection_id: ConnectionId,

    pub data: T,
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
