use std::ops::ControlFlow;

use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, TermBuilderImpl, World},
    macros::{system, Component},
    prelude::{Module, SystemAPI},
};
use geometry::aabb::Aabb;
use hyperion::{
    glam::Vec3,
    net::{agnostic, Compose},
    simulation::{
        aabb, block_bounds, blocks::Blocks, event::HitGroundEvent, EntitySize, Gamemode,
        MovementTracking, Player, Position,
    },
    storage::EventQueue,
    BlockKind,
};
use valence_server::{
    block::{PropName, PropValue},
    ident,
};

use crate::{damage_player, is_invincible, DamageCause, DamageType};

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

                    entity.get::<(&Position, &Gamemode)>(|(position, gamemode)| {
                        if is_invincible(&gamemode.current) {
                            return;
                        }

                        damage_player(
                            &entity,
                            damage,
                            DamageCause::new(DamageType::Fall),
                            compose,
                            system,
                        );

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
                            .broadcast_local(&sound, position.to_chunk(), system)
                            .send()
                            .unwrap();
                    });
                }
            },
        );

        #[allow(clippy::excessive_nesting)]
        system!("natural block damage", world, &Compose($), &Blocks($), &Position, &EntitySize, &Gamemode, &MovementTracking)
            .with::<Player>()
            .each_iter(|it, row, (compose, blocks, position, size, gamemode, movement)| {
                if is_invincible(&gamemode.current) {
                    return;
                }

                let system = it.system();
                let entity = it.entity(row);

                let (min, max) = block_bounds(**position, *size);
                let min = min.with_y(min.y-1);
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
                                    damage_player(&entity, 1., DamageCause::new(DamageType::Cactus), compose, system);
                                }
                                BlockKind::MagmaBlock => {
                                    if position.y > pos.y {
                                        damage_player(&entity, 1., DamageCause::new(DamageType::HotFloor), compose, system);
                                    }
                                }
                                _ => {}
                            }
                            return ControlFlow::Break(());
                        }
                    }

                    let aabb = Aabb::new(Vec3::ZERO, Vec3::ONE).move_by(pos);

                    if Aabb::overlap(&aabb, &bounding_box).is_some() {
                        match kind {
                            BlockKind::Fire => {
                                damage_player(&entity, 1., DamageCause::new(DamageType::InFire), compose, system);
                            }
                            BlockKind::SoulFire => {
                                damage_player(&entity, 2., DamageCause::new(DamageType::InFire), compose, system);
                            }
                            BlockKind::SweetBerryBush => {
                                let grown = block.get(PropName::Age)
                                    .is_some_and(|x| x != PropValue::_0);
                                let delta_x = (f64::from(position.x) - f64::from(movement.last_tick_position.x)).abs();
                                let delta_y = (f64::from(position.y) - f64::from(movement.last_tick_position.y)).abs();

                                if grown && delta_x >= 0.003_000_000_026_077_032 && delta_y >= 0.003_000_000_026_077_032 && movement.last_tick_position != **position {
                                    damage_player(&entity, 1., DamageCause::new(DamageType::SweetBerryBush), compose, system);
                                }
                            }
                            _ => {}
                        }
                    }
                    ControlFlow::Continue(())
                });
            });
    }
}

#[must_use]
pub const fn is_harmful_block(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Lava
            | BlockKind::Cactus
            | BlockKind::MagmaBlock
            | BlockKind::SweetBerryBush
            | BlockKind::SoulFire
            | BlockKind::Fire
    )
}
