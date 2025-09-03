use bytes::Bytes;
use hyperion_proto::{ArchivedSetReceiveBroadcasts, ArchivedShutdown};
use rustc_hash::FxBuildHasher;
use tracing::{error, instrument, warn};

use crate::{data::PlayerHandle, server_sender::ServerSender};

#[derive(Clone)]
pub struct Egress {
    // todo: can we do some type of EntityId and SlotMap
    pub(crate) player_registry: &'static papaya::HashMap<u64, PlayerHandle, FxBuildHasher>,
    pub(crate) server_sender: ServerSender,
}

impl Egress {
    #[must_use]
    pub const fn new(
        player_registry: &'static papaya::HashMap<u64, PlayerHandle, FxBuildHasher>,
        server_sender: ServerSender,
    ) -> Self {
        Self {
            player_registry,
            server_sender,
        }
    }

    #[instrument(skip_all)]
    pub fn unicast(&self, stream: u64, data: Bytes) {
        let players = self.player_registry.pin();

        let Some(player) = players.get(&stream) else {
            // expected to still happen infrequently
            warn!("Player not found for id {stream:?}");
            return;
        };

        // todo: handle error; kick player if cannot send (buffer full)
        if let Err(e) = player.send(data) {
            warn!("Failed to send data to player: {:?}", e);
            player.shutdown();
        }
    }

    #[instrument(skip_all)]
    pub fn handle_set_receive_broadcasts(&self, pkt: &ArchivedSetReceiveBroadcasts) {
        let player_registry = self.player_registry;
        let players = player_registry.pin();
        let Ok(stream) = rkyv::deserialize::<u64, rkyv::rancor::Error>(&pkt.stream) else {
            return;
        };

        let Some(player) = players.get(&stream) else {
            error!("Player not found for stream {stream:?}");
            return;
        };

        player.enable_receive_broadcasts();
    }

    #[instrument(skip_all)]
    pub fn handle_shutdown(&self, pkt: &ArchivedShutdown) {
        let player_registry = self.player_registry;
        let players = player_registry.pin();
        let Ok(stream) = rkyv::deserialize::<u64, rkyv::rancor::Error>(&pkt.stream) else {
            return;
        };

        if let Some(result) = players.get(&stream) {
            result.shutdown();
        } else {
            error!("Player not found for stream {stream:?}");
        }
    }
}
