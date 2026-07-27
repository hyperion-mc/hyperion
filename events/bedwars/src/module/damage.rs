use std::sync::LazyLock;

use flecs_ecs::{
    core::{EntityViewGet, World},
    macros::{Component, system},
    prelude::{Module, SystemAPI},
};
use hyperion::{
    hyperion_minecraft_proto::{
        RegistryId, generated::packet_id::play::clientbound::PacketId,
        packets::play::entity::DamageEvent,
    },
    net::{
        Compose, ConnectionId, agnostic,
        protocol::{registries, send},
    },
    simulation::{Position, event::HitGroundEvent, metadata::living_entity::Health},
    storage::EventQueue,
};
use hyperion_utils::EntityExt;
use valence_server::ident;

/// `minecraft:fall`'s id in the damage type registry this server synchronises.
///
/// The id is positional in the registry the server sent at configuration time,
/// so it is looked up rather than written down: a reordered
/// [`registries::DAMAGE_TYPE`] would otherwise silently start rendering fall
/// damage as something else.
static FALL: LazyLock<RegistryId> = LazyLock::new(|| {
    RegistryId(
        registries::DAMAGE_TYPE
            .id_of("minecraft:fall")
            .expect("minecraft:fall is a vanilla damage type"),
    )
});

#[derive(Component)]
pub struct DamageModule {}

impl Module for DamageModule {
    fn module(world: &World) {
        system!(
            "apply natural damages",
            world,
            &mut EventQueue<HitGroundEvent>,
            &Compose
        )
        .each_iter(|it, _, (event_queue, compose)| {
            let world = it.world();

            for event in event_queue.drain() {
                if event.fall_distance <= 3. {
                    continue;
                }

                let entity = event.client.entity_view(world);
                // TODO account for armor/effects and gamemode
                let damage = event.fall_distance.floor() - 3.;

                if damage <= 0. {
                    continue;
                }

                entity.get::<(&mut Health, &ConnectionId, &Position)>(
                    |(health, connection, position)| {
                        health.damage(damage);

                        // No cause and no direct entity: the ground is not an
                        // entity, and `writeOptionalEntityId` puts absence on
                        // the wire as the zero this `None` becomes.
                        let pkt_damage_event = DamageEvent {
                            entity_id: entity.minecraft_id(),
                            source_type: *FALL,
                            source_cause_id: None,
                            source_direct_id: None,
                            source_position: None,
                        };

                        let sound = agnostic::sound(
                            if event.fall_distance > 7. {
                                ident!("minecraft:entity.player.big_fall")
                            } else {
                                ident!("minecraft:entity.player.small_fall")
                            },
                            **position,
                        )
                        .volume(1.)
                        .pitch(1.)
                        .seed(fastrand::i64(..))
                        .build();

                        send(
                            compose,
                            *connection,
                            PacketId::DamageEvent.to_raw(),
                            &pkt_damage_event,
                        )
                        .unwrap();
                        compose
                            .broadcast_local(&sound, position.to_chunk())
                            .send()
                            .unwrap();
                    },
                );
            }
        });
    }
}
