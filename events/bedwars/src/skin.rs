use flecs_ecs::{
    core::{Entity, EntityViewGet, SystemAPI, World},
    macros::{Component, system},
    prelude::Module,
};
use hyperion::{
    egress::player_join::{PlayerInfoActions, PlayerList, PlayerListEntry, SkinProperty},
    hyperion_minecraft_proto::{
        Uuid as ProtoUuid,
        generated::packet_id::play::clientbound::PacketId,
        packets::{
            play::clientbound::{PlayerInfoRemove, RemoveEntities},
            play_login::Respawn,
        },
    },
    net::{
        Compose, ConnectionId, DataBundle,
        protocol::{Clientbound, join},
    },
    simulation::{event, skin::PlayerSkin},
    storage::EventQueue,
    uuid::Uuid,
};
use hyperion_utils::EntityExt;
use tracing::debug;

#[derive(Component)]
pub struct SkinModule;

impl Module for SkinModule {
    fn module(world: &World) {
        system!("set_skin", world, &mut EventQueue<event::SetSkin>, &Compose).each_iter(
            |it, _, (event_queue, compose)| {
                let world = it.world();
                for event in event_queue.drain() {
                    debug!("got {event:?}");
                    event
                        .by
                        .entity_view(world)
                        .get::<(&ConnectionId, &hyperion::simulation::Uuid)>(|(io, uuid)| {
                            on_set_skin(event.by, compose, uuid.0, event.skin, *io).unwrap();
                        });
                }
            },
        );
    }
}

/// Re-send the player to every client so a new skin takes effect.
///
/// A skin lives in the profile the tab list carries, and a client only reads
/// it when the profile arrives, so changing it means removing the entry and
/// the entity and adding both back.
///
/// # Errors
/// Returns an error when a packet fails to encode or when the registries carry
/// no overworld dimension type.
fn on_set_skin(
    id: Entity,
    compose: &Compose,
    uuid: Uuid,
    skin: PlayerSkin,
    io: ConnectionId,
) -> anyhow::Result<()> {
    let mut bundle = DataBundle::new(compose);

    // Remove player info
    let remove_info = PlayerInfoRemove(vec![ProtoUuid(uuid.as_u128())]);
    bundle.add_packet(Clientbound::new(
        PacketId::PlayerInfoRemove.to_raw(),
        &remove_info,
    ))?;

    // Destroy player entity
    let remove_entity = RemoveEntities(vec![id.minecraft_id()]);
    bundle.add_packet(Clientbound::new(
        PacketId::RemoveEntities.to_raw(),
        &remove_entity,
    ))?;

    // Add player back with new skin. Only `ADD_PLAYER` is set, so the entry's
    // other fields are not on the wire and are left at their defaults.
    bundle.add_packet(&PlayerList {
        actions: PlayerInfoActions::ADD_PLAYER,
        entries: vec![PlayerListEntry {
            uuid,
            username: "Player".to_owned(),
            properties: vec![SkinProperty {
                name: "textures".to_owned(),
                value: skin.textures,
                signature: Some(skin.signature),
            }],
            ..PlayerListEntry::default()
        }],
    })?;

    // Respawn player
    let respawn = Respawn {
        spawn_info: join::spawn_info()?,
        // Zero is what the 1.20.1 `copy_metadata: false` this replaces meant.
        // Keeping attributes or entity data across a cosmetic respawn is a
        // behaviour change, not a port, so it is not made here.
        data_to_keep: 0,
    };
    bundle.add_packet(Clientbound::new(PacketId::Respawn.to_raw(), &respawn))?;

    bundle.unicast(io)
}
