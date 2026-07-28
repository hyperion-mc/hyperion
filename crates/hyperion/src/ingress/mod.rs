use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::{
    Uuid as ProtoUuid,
    generated::packet_id::play::clientbound::PacketId,
    packets::play::clientbound::{PlayerInfoRemove, RemoveEntities},
};
use hyperion_utils::EntityExt;
use tracing::{error, info, info_span};

use crate::{
    Shutdown,
    ingress::decode::DecodeModule,
    net::{Compose, protocol::Clientbound},
    simulation::{IgnMap, PacketState, Uuid},
};

pub mod decode;

/// This marks players who have already been disconnected and about to be destructed. This component should not be
/// added to an entity to disconnect a player. Use [`crate::net::IoBuf::shutdown`] instead.
#[derive(Component, Debug)]
pub struct PendingRemove;

/// The data sent to clients which ping the server without logging in.
#[derive(Component)]
pub struct ServerPingResponse {
    pub description: String,
    pub max_players: u32,
}

impl Default for ServerPingResponse {
    fn default() -> Self {
        Self {
            description: String::from(
                "Getting 10k Players to PvP at Once on a Minecraft Server to Break the Guinness \
                 World Record",
            ),
            max_players: 12_000,
        }
    }
}

fn remove_player_from_visibility(world: &World) {
    world
        .observer_named::<flecs::OnRemove, ()>("remove_player_from_visibility")
        .with_enum(PacketState::Play)
        .each_entity(|entity, ()| {
            let world = entity.world();

            let Some(uuid) = entity.try_get::<&Uuid>(|uuid| uuid.0) else {
                error!("failed to send player remove packet: player has no uuid");
                return;
            };

            world.get::<&Compose>(|compose| {
                // destroy
                let pkt = RemoveEntities(vec![entity.id().minecraft_id()]);

                if let Err(e) = compose
                    .broadcast(Clientbound::new(PacketId::RemoveEntities.to_raw(), &pkt))
                    .send()
                {
                    error!("failed to send player remove packet: {e}");
                    return;
                }

                let pkt = PlayerInfoRemove(vec![ProtoUuid(uuid.as_u128())]);

                if let Err(e) = compose
                    .broadcast(Clientbound::new(PacketId::PlayerInfoRemove.to_raw(), &pkt))
                    .send()
                {
                    error!("failed to send player remove packet: {e}");
                }
            });
        });
}

#[derive(Component)]
pub struct IngressModule;

impl Module for IngressModule {
    fn module(world: &World) {
        world.component::<PendingRemove>();
        world
            .component::<ServerPingResponse>()
            .add_trait::<flecs::Singleton>();
        world.set(ServerPingResponse::default());

        world.import::<DecodeModule>();

        system!("shutdown", world, &Shutdown)
            .kind(id::<flecs::pipeline::OnLoad>())
            .each_iter(|it, _, shutdown| {
                let world = it.world();
                if shutdown.value.load(std::sync::atomic::Ordering::Relaxed) {
                    info!("shutting down");
                    world.quit();
                }
            });

        system!("update_ign_map", world, &mut IgnMap)
            .kind(id::<flecs::pipeline::OnLoad>())
            .each_iter(|_, _, ign_map| {
                let span = info_span!("update_ign_map");
                let _enter = span.enter();
                ign_map.update();
            });

        world.import::<crate::net::protocol::ProtocolModule>();

        remove_player_from_visibility(world);

        world
            .system_named::<()>("remove_player")
            .kind(id::<flecs::pipeline::PostLoad>())
            .with(id::<PendingRemove>())
            .each_entity(|entity, ()| {
                entity.destruct();
            });
    }
}
