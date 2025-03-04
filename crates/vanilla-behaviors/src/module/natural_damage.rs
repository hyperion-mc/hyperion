use std::ops::ControlFlow;

use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, TermBuilderImpl, World},
    macros::{system, Component},
    prelude::{Module, SystemAPI},
};
use geometry::aabb::Aabb;
use hyperion::{
    glam::Vec3,
    net::{agnostic, Compose, ConnectionId},
    simulation::{
        aabb, block_bounds, blocks::Blocks, event::HitGroundEvent, EntitySize, Gamemode, Player,
        Position,
    },
    storage::EventQueue,
    BlockKind,
};
use hyperion_utils::EntityExt;
use valence_protocol::{packets::play, VarInt};
use valence_server::{ident, GameMode};

use crate::damage_player;

#[derive(Component)]
pub struct NaturalDamageModule {}

impl Module for NaturalDamageModule {
    fn module(world: &World) {
        system!("fall damage", world, &mut EventQueue<HitGroundEvent>($), &Compose($)).each_iter(
            |it, _, (event_queue, compose)| {
                let world = it.world();
                let system = it.system();

                for event in event_queue.drain() {
                    if event.fall_distance <= 3. {
                        continue;
                    }

                    let entity = event.client.entity_view(world);
                    // TODO account for armor/effects
                    let damage = event.fall_distance.floor() - 3.;

                    if damage <= 0. {
                        continue;
                    }

                    entity.get::<(&ConnectionId, &Position, &Gamemode)>(
                        |(connection, position, gamemode)| {
                            if gamemode.current != GameMode::Survival
                                && gamemode.current != GameMode::Adventure
                            {
                                return;
                            }

                            damage_player(&entity, damage, compose);

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
            },
        );

        system!("natural block damage", world, &Compose($), &Blocks($), &Position, &EntitySize)
            .with::<Player>()
            .each_iter(|it, row, (compose, blocks, position, size)| {
                let world = it.world();
                let system = it.system();
                let entity = it.entity(row);

                let (min, max) = block_bounds(**position, *size);

                let bounding_box = aabb(**position, *size);

                blocks.get_blocks(min, max, |pos, block| {
                    let pos = Vec3::new(pos.x as f32, pos.y as f32, pos.z as f32);
                    let kind = block.to_kind();

                    if !is_harmful_block(kind) {
                        return ControlFlow::Continue(());
                    }

                    for aabb in block.collision_shapes() {
                        let aabb = Aabb::new(aabb.min().as_vec3(), aabb.max().as_vec3());
                        let aabb = aabb.move_by(pos);

                        if bounding_box.collides(&aabb) {
                            match kind {
                                BlockKind::Cactus => {
                                    damage_player(&entity, 1., compose);
                                }
                                _ => {}
                            }
                            return ControlFlow::Break(false);
                        }
                    }

                    ControlFlow::Continue(())
                });
            });
    }
}

pub fn is_harmful_block(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Lava | BlockKind::Cactus | BlockKind::MagmaBlock | BlockKind::SweetBerryBush
    )
}
