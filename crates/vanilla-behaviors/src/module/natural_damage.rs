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
        aabb, block_bounds, blocks::Blocks, event::HitGroundEvent, metadata::entity::EntityFlags,
        BurningState, EntitySize, Gamemode, MovementTracking, Player, Position,
    },
    storage::EventQueue,
    BlockKind,
};
use hyperion_utils::structures::{DamageCause, DamageType};
use tracing::warn;
use valence_protocol::{packets::play, Sound};
use valence_server::{
    block::{PropName, PropValue},
    text::IntoText,
};

use crate::{damage_player, is_invincible};

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
                                Sound::EntityPlayerBigFall.to_ident()
                            } else {
                                Sound::EntityPlayerSmallFall.to_ident()
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
        system!("natural block damage", world, &Compose($), &Blocks($), &Position, &EntitySize, &Gamemode, &MovementTracking, &mut BurningState, &EntityFlags)
            .with::<Player>()
            .each_iter(|it, row, (compose, blocks, position, size, gamemode, movement, burning, flags)| {
                if is_invincible(&gamemode.current) {
                    return;
                }

                let system = it.system();
                let entity = it.entity(row);

                let (min, max) = block_bounds(**position, *size);
                let min = min.with_y(min.y-1);
                let bounding_box = aabb(**position, *size);
                let mut in_fire_source = false;
                // water, powder snow, bubble column + TODO rain
                let mut in_extinguisher = false;

                if burning.fire_ticks_left > 0 {
                    if burning.immune {
                        burning.fire_ticks_left = (burning.fire_ticks_left - 4).max(0);
                    }else {
                        if !burning.in_lava && burning.fire_ticks_left % 20 == 0 {
                            damage_player(&entity, 1., DamageCause::new(DamageType::OnFire), compose, system);
                        }
                        burning.fire_ticks_left -= 1;
                    }
                }

                burning.in_lava = false;

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
                                    if position.y > pos.y && !burning.immune {
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
                                in_fire_source = true;
                                if !burning.immune {
                                    damage_player(&entity, 1., DamageCause::new(DamageType::InFire), compose, system);
                                    burning.fire_ticks_left += 1;
                                    if burning.fire_ticks_left == 0 {
                                        burning.burn_for_seconds(8);
                                    }
                                }
                            }
                            BlockKind::SoulFire => {
                                in_fire_source = true;
                                if !burning.immune {
                                    damage_player(&entity, 2., DamageCause::new(DamageType::InFire), compose, system);
                                    burning.fire_ticks_left += 1;
                                    if burning.fire_ticks_left == 0 {
                                        burning.burn_for_seconds(8);
                                    }
                                }
                            }
                            BlockKind::Lava => {
                                in_fire_source = true;
                                burning.in_lava = true;
                                if !burning.immune {
                                    if damage_player(&entity, 4., DamageCause::new(DamageType::InFire), compose, system) {
                                        let sound = agnostic::sound(
                                            Sound::EntityGenericBurn.to_ident(),
                                            **position,
                                        )
                                        .volume(0.4)
                                        .pitch(2.) // 2.0F + this.random.nextFloat() * 0.4F
                                        .seed(fastrand::i64(..))
                                        .build();

                                        if compose.broadcast_local(&sound, position.to_chunk(), system).send().is_err() {
                                            warn!("Failed to send burn sound to players");
                                        }
                                    }
                                    burning.burn_for_seconds(15);
                                }
                            }
                            BlockKind::SweetBerryBush => {
                                let grown = block.get(PropName::Age).is_some_and(|x| x != PropValue::_0);
                                let delta_x = (f64::from(position.x) - f64::from(movement.last_tick_position.x)).abs();
                                let delta_y = (f64::from(position.y) - f64::from(movement.last_tick_position.y)).abs();

                                if grown && (delta_x >= 0.003_000_000_026_077_032 || delta_y >= 0.003_000_000_026_077_032) {
                                    damage_player(&entity, 1., DamageCause::new(DamageType::SweetBerryBush), compose, system);
                                }
                            }
                            BlockKind::Water | BlockKind::BubbleColumn | BlockKind::PowderSnow => {
                                in_extinguisher = true;
                            }
                            _ => {}
                        }
                    }
                    ControlFlow::Continue(())
                });

                if burning.fire_ticks_left > 0 && in_extinguisher {
                    if !in_fire_source {
                        let sound = agnostic::sound(
                            Sound::EntityGenericExtinguishFire.to_ident(),
                            **position,
                        )
                        .volume(0.7)
                        .pitch(1.)
                        .seed(fastrand::i64(..))
                        .build();

                        if compose.broadcast_local(&sound, position.to_chunk(), system).send().is_err() {
                            warn!("Failed to send extinguish sound to players");
                        }
                    }
                    burning.fire_ticks_left = -20; // -1 for every entities except players
                }else if !in_fire_source && burning.fire_ticks_left <= 0 {
                    burning.fire_ticks_left = -20; // -1 for every entities except players
                }

                if burning.fire_ticks_left > 0 {
                    entity.set(*flags | EntityFlags::ON_FIRE);
                } else {
                    entity.set(*flags & !EntityFlags::ON_FIRE);
                }
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
            | BlockKind::Water
            | BlockKind::BubbleColumn
            | BlockKind::PowderSnow
    )
}
