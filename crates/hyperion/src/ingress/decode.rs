// It is easier, simpler, and faster in terms of compilation speed to use the packet and state names as-is from
// for_each_state than it is to use the paste macro to convert them to the appropriate casing
#![allow(nonstandard_style)]

use bevy::{ecs::system::SystemParam, prelude::*};
use hyperion_packet_macros::*;
use paste::paste;
use tracing::error;
use valence_protocol::Packet as _;

use crate::{
    net::{Compose, ConnectionId, PacketDecoder, decoder::BorrowedPacketFrame, packet_channel},
    simulation::{
        packet::{self, Packet},
        packet_state,
    },
};

fn try_next_frame(
    compose: &Compose,
    connection_id: ConnectionId,
    decoder: &mut PacketDecoder,
    receiver: &mut packet_channel::Receiver,
) -> Option<BorrowedPacketFrame> {
    let raw_packet = receiver.try_recv()?;
    let bump = compose.bump();
    match decoder.try_next_packet(bump, &raw_packet) {
        Ok(Some(packet)) => Some(packet),
        Ok(None) => None,
        Err(e) => {
            error!("failed to decode packet: {e}");
            compose.io_buf().shutdown(connection_id);
            None
        }
    }
}

mod writers {
    use super::*;
    for_each_state! {
        #{
            #for_each_packet! {
                #[derive(SystemParam)]
                pub struct #state<'w> {
                    #{pub #packet_name: EventWriter<'w, packet::#state::#packet_name>,}
                }
            }
        }
    }
}

/// Whether the protocol may transition to another state after a packet in one of these states are
/// sent
mod may_transition {
    pub const handshake: bool = true;
    pub const status: bool = false;
    pub const login: bool = true;
    pub const play: bool = false;
}

mod decoders {
    use super::*;
    for_each_state! {
        #{
            pub fn #state(
                query: Query<
                '_,
                '_,
                (
                    Entity,
                    &ConnectionId,
                    &mut PacketDecoder,
                    &mut packet_channel::Receiver,
                ),
                paste! { With<packet_state::[< #state:camel >]> }
                >,
                compose: Res<'_, Compose>,
                mut writers: super::writers::#state<'_>,
            ) {
                let compose = compose.into_inner();
                for (sender, &connection_id, decoder, receiver) in query {
                    let decoder = decoder.into_inner();
                    let receiver = receiver.into_inner();
                    loop {
                        let Some(frame) = try_next_frame(
                            compose,
                            connection_id,
                            decoder,
                            receiver,
                        ) else {
                            break;
                        };

                        let frame_id = frame.id;

                        #for_each_packet! {
                            let result: anyhow::Result<()> = match frame_id {
                                #{
                                    #valence_packet::ID => {
                                        match frame.decode::<#static_valence_packet>() {
                                            Ok(data) => {
                                                writers.#packet_name.write(Packet::new(
                                                    sender,
                                                    connection_id,
                                                    data
                                                ));
                                                Ok(())
                                            },
                                            Err(e) => Err(e)
                                        }
                                    },
                                }
                                _ => {
                                    Err(anyhow::Error::msg("invalid packet id"))
                                }
                            };
                        }

                        if let Err(e) = result {
                            // The call to error! is placed outside of the match statement to help reduce
                            // compile times by reducing code duplication from the expansion of the error!
                            // macro
                            error!("error while decoding packet (id: {frame_id}): {e}");
                            compose.io_buf().shutdown(connection_id);
                            break;
                        }

                        if may_transition::#state {
                            // The packet handler for this packet might change the player to
                            // another packet state, so more packets cannot be decoded at this
                            // moment
                            break;
                        }
                    }
                }
            }
        }
    }
}

pub struct DecodePlugin;

impl Plugin for DecodePlugin {
    fn build(&self, app: &mut App) {
        for_each_state! {
            app.add_systems(
                FixedPreUpdate, (
                    #{
                        decoders::#state,
                    }
                )
            );
        }
    }
}
