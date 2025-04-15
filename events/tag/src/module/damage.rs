use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, TermBuilderImpl, World},
    macros::{Component, system},
    prelude::{Module, SystemAPI},
};
use hyperion::{
    net::{Compose, ConnectionId, agnostic},
    simulation::{Position, event::HitGroundEvent, metadata::living_entity::Health},
    storage::EventQueue,
};
use hyperion_utils::EntityExt;
use valence_protocol::{VarInt, packets::play};
use valence_server::ident;

#[derive(Component)]
pub struct DamageModule {}

impl Module for DamageModule {
    fn module(world: &World) {
        system!("fall_damage", world, &mut EventQueue<HitGroundEvent>($), &Compose($))
            .each_iter(|it, _, (event_queue, compose)| {
                let world = it.world();
                let system = it.system();

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

                            let pkt_damage_event = play::EntityDamageS2c {
                                entity_id: VarInt(entity.minecraft_id()),
                                source_cause_id: VarInt(0),
                                source_direct_id: VarInt(0),
                                source_type_id: VarInt(10), // 10 = fall damage
                                source_pos: Option::None,
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

                            compose
                                .unicast(&pkt_damage_event, *connection, system)
                                .unwrap();
                            compose
                                .broadcast_local(&sound, position.to_chunk(), system)
                                .send()
                                .unwrap();
                        },
                    );
                }
            });
    }
}
