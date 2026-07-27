use std::borrow::Cow;

use flecs_ecs::{
    core::{
        Builder, ComponentOrPairId, EntityView, EntityViewGet, QueryAPI, SystemAPI, TableIter,
        World, WorldGet, flecs, id,
    },
    macros::{Component, system},
    prelude::Module,
};
use glam::IVec3;
use hyperion::{
    BlockKind, Prev,
    net::{Compose, ConnectionId, agnostic},
    runtime::AsyncRuntime,
    simulation::{
        PendingTeleportation, Player, Position, Velocity, Xp, Yaw,
        blocks::Blocks,
        event::{self, ClientStatusCommand, ClientStatusEvent},
        handlers::PacketSwitchQuery,
        metadata::{entity::Pose, living_entity::Health},
        packet::HandlerRegistry,
    },
    storage::EventQueue,
    valence_protocol::{
        ItemKind, ItemStack, Particle, VarInt, ident,
        math::{DVec3, Vec3},
        packets::play::{self, entity_attributes_s2c::AttributeProperty},
        text::IntoText,
    },
};
use hyperion_inventory::PlayerInventory;
use hyperion_utils::EntityExt;
use tracing::info_span;

use super::spawn::{avoid_blocks, find_spawn_position, is_valid_spawn_block};
use crate::Team;

#[derive(Component)]
pub struct AttackModule;

#[derive(Component, Default, Copy, Clone, Debug)]
#[flecs(meta)]
pub struct ImmuneUntil {
    tick: i64,
}

#[derive(Component, Default, Copy, Clone, Debug)]
#[flecs(meta)]
pub struct Armor {
    pub armor: f32,
}

// Used as a component only for commands, does not include armor or weapons
#[derive(Component, Default, Copy, Clone, Debug)]
#[flecs(meta)]
pub struct CombatStats {
    pub armor: f32,
    pub armor_toughness: f32,
    pub damage: f32,
    pub protection: f32,
}

#[allow(clippy::cast_possible_truncation)]
impl Module for AttackModule {
    #[allow(clippy::excessive_nesting)]
    #[allow(clippy::cast_sign_loss)]
    fn module(world: &World) {
        world.component::<ImmuneUntil>().meta();
        world.component::<Armor>().meta();
        world.component::<CombatStats>().meta();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, ImmuneUntil)>()
            .add_trait::<(flecs::With, CombatStats)>()
            .add_trait::<(flecs::With, Armor)>();

        // TODO: This code should be split between melee attacks and bow attacks
        system!("handle_attacks", world, &mut EventQueue<event::AttackEntity>, &Compose)

            .each_iter(
            move |it: TableIter<'_, false>,
                _,
                (event_queue, compose): (
                    &mut EventQueue<event::AttackEntity>,
                    &Compose,
                )| {
                    const IMMUNE_TICK_DURATION: i64 = 10;

                    let span = info_span!("handle_attacks");
                    let _enter = span.enter();

                    let current_tick = compose.global().tick;

                    let world = it.world();

                    for event in event_queue.drain() {
                        let target = world.entity_from_id(event.target);
                        let origin = world.entity_from_id(event.origin);
                        let critical_hit = can_critical_hit(origin);
                        origin.get::<(&ConnectionId, &Position, &mut Armor, &CombatStats, &PlayerInventory, &Team, &mut Xp)>(|(origin_connection, origin_pos, origin_armor, from_stats, from_inventory, origin_team, origin_xp)| {
                            let damage = from_stats.damage + calculate_stats(from_inventory, critical_hit).damage;
                            target.try_get::<(
                                &ConnectionId,
                                Option<&mut ImmuneUntil>,
                                &mut Health,
                                &mut Position,
                                &Yaw,
                                &CombatStats,
                                &PlayerInventory,
                                &Team,
                                &mut Pose,
                                &mut Xp,
                                &mut Velocity
                            )>(
                                |(target_connection, immune_until, health, target_position, target_yaw, stats, target_inventory, target_team, target_pose, target_xp, target_velocity)| {
                                    if let Some(immune_until) = immune_until {
                                        if immune_until.tick > current_tick {
                                            return;
                                        }
                                        immune_until.tick = current_tick + IMMUNE_TICK_DURATION;
                                    }

                                    if target_team == origin_team {
                                        let msg = "§cCannot attack teammates";
                                        let pkt_msg = play::GameMessageS2c {
                                            chat: msg.into_cow_text(),
                                            overlay: false,
                                        };

                                        compose.unicast(&pkt_msg, *origin_connection).unwrap();
                                        return;
                                    }

                                    let calculated_stats = calculate_stats(target_inventory, critical_hit);
                                    let armor = stats.armor + calculated_stats.armor;
                                    let toughness = stats.armor_toughness + calculated_stats.armor_toughness;
                                    let protection = stats.protection + calculated_stats.protection;

                                    let damage_after_armor = get_damage_left(damage, armor, toughness);
                                    let damage_after_protection = get_inflicted_damage(damage_after_armor, protection);

                                    health.damage(damage_after_protection);

                                    let delta_x: f64 = f64::from(target_position.x - origin_pos.x);
                                    let delta_z: f64 = f64::from(target_position.z - origin_pos.z);

                                    // Seems that MC generates a random delta if the damage source is too close to the target
                                    // let's ignore that for now
                                    let pkt_hurt = play::DamageTiltS2c {
                                        entity_id: VarInt(target.minecraft_id()),
                                        yaw: delta_z.atan2(delta_x).mul_add(57.295_776_367_187_5_f64, -f64::from(**target_yaw)) as f32
                                    };
                                    // EntityDamageS2c: display red outline when taking damage (play arrow hit sound?)
                                    let pkt_damage_event = play::EntityDamageS2c {
                                        entity_id: VarInt(target.minecraft_id()),
                                        source_cause_id: VarInt(origin.minecraft_id() + 1), // this is an OptVarint
                                        source_direct_id: VarInt(origin.minecraft_id() + 1), // if hit by a projectile, it should be the projectile's entity id
                                        source_type_id: VarInt(31), // 31 = player_attack
                                        source_pos: None
                                    };
                                    let sound = agnostic::sound(
                                        if critical_hit { ident!("minecraft:entity.player.attack.crit") } else { ident!("minecraft:entity.player.attack.knockback") },
                                        **target_position,
                                    ).volume(1.)
                                    .pitch(1.)
                                    .seed(fastrand::i64(..))
                                    .build();

                                    compose.unicast(&pkt_hurt, *target_connection).unwrap();

                                    if critical_hit {
                                    let particle_pkt = play::ParticleS2c {
                                            particle: Cow::Owned(Particle::Crit),
                                            long_distance: true,
                                            position: target_position.as_dvec3() + DVec3::new(0.0, 1.0, 0.0),
                                            max_speed: 0.5,
                                            count: 100,
                                            offset: Vec3::new(0.5, 0.5, 0.5),
                                        };

                                        // origin is excluded because the crit particles are
                                        // already generated on the client side of the attacker
                                        compose.broadcast(&particle_pkt).exclude(*origin_connection).send().unwrap();
                                    }

                                    if health.is_dead() {
                                        let attacker_name = origin.name();
                                        // Even if enable_respawn_screen is false, the client needs this to send ClientCommandC2s and initiate its respawn
                                        let pkt_death_screen = play::DeathMessageS2c {
                                            player_id: VarInt(target.minecraft_id()),
                                            message: format!("You were killed by {attacker_name}").into_cow_text()
                                        };
                                        compose.unicast(&pkt_death_screen, *target_connection).unwrap();
                                    }
                                    compose.broadcast(&sound).send().unwrap();
                                    compose.broadcast(&pkt_damage_event).send().unwrap();

                                    if health.is_dead() {
                                        // Create particle effect at the attacker's position
                                        let particle_pkt = play::ParticleS2c {
                                            particle: Cow::Owned(Particle::Explosion),
                                            long_distance: true,
                                            position: target_position.as_dvec3() + DVec3::new(0.0, 1.0, 0.0),
                                            max_speed: 0.5,
                                            count: 100,
                                            offset: Vec3::new(0.5, 0.5, 0.5),
                                        };

                                        // Add a second particle effect for more visual impact
                                        let particle_pkt2 = play::ParticleS2c {
                                            particle: Cow::Owned(Particle::DragonBreath),
                                            long_distance: true,
                                            position: target_position.as_dvec3() + DVec3::new(0.0, 1.5, 0.0),
                                            max_speed: 0.2,
                                            count: 75,
                                            offset: Vec3::new(0.3, 0.3, 0.3),
                                        };
                                        let pkt_entity_status = play::EntityStatusS2c {
                                            entity_id: target.minecraft_id(),
                                            entity_status: 3
                                        };

                                        let origin_entity_id = origin.minecraft_id();

                                        origin_armor.armor += 1.0;
                                        let pkt = play::EntityAttributesS2c {
                                            entity_id: VarInt(origin_entity_id),
                                            properties: vec![
                                                AttributeProperty {
                                                    key: ident!("minecraft:generic.armor"),
                                                    value: origin_armor.armor.into(),
                                                    modifiers: vec![],
                                                }
                                            ],
                                        };

                                        let entities_to_remove = [VarInt(target.minecraft_id())];
                                        let pkt_remove_entities = play::EntitiesDestroyS2c {
                                            entity_ids: Cow::Borrowed(&entities_to_remove)
                                        };

                                        *target_pose = Pose::Dying;
                                        target.modified(id::<Pose>());
                                        compose.broadcast(&pkt).send().unwrap();
                                        compose.broadcast(&particle_pkt).send().unwrap();
                                        compose.broadcast(&particle_pkt2).send().unwrap();
                                        compose.broadcast(&pkt_entity_status).send().unwrap();
                                        compose.broadcast(&pkt_remove_entities).send().unwrap();

                                        target.set::<Team>(*origin_team);

                                        origin_xp.amount = (f32::from(target_xp.amount)*0.5) as u16;
                                        target_xp.amount = (f32::from(target_xp.amount)/3.) as u16;

                                        return;
                                    }

                                    // Calculate velocity change based on attack direction
                                    let this = **target_position;
                                    let other = **origin_pos;

                                    let dir = (this - other).normalize();

                                    let knockback_xz = 8.0;
                                    let knockback_y = 6.432;

                                    let new_vel = Vec3::new(
                                        dir.x * knockback_xz / 20.0,
                                        knockback_y / 20.0,
                                        dir.z * knockback_xz / 20.0
                                    );

                                    target_velocity.0 += new_vel;

                                    // https://github.com/valence-rs/valence/blob/8f3f84d557dacddd7faddb2ad724185ecee2e482/examples/ctf.rs#L987-L989
                                },
                            );
                        });
                    }
                },
            );

        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |client_status: &ClientStatusEvent, query: &mut PacketSwitchQuery<'_>| {
                    if client_status.status == ClientStatusCommand::RequestStats {
                        return Ok(());
                    }

                    let client = client_status.client.entity_view(query.world);

                    client.get::<&Team>(|team| {
                        let mut pos_vec = vec![];

                        query
                            .world
                            .query::<(&Position, &Team)>()
                            .build()
                            .each_entity(|candidate, (candidate_pos, candidate_team)| {
                                if team != candidate_team || candidate == client {
                                    return;
                                }
                                pos_vec.push(*candidate_pos);
                            });

                        let respawn_pos = if let Some(random_mate) = fastrand::choice(pos_vec) {
                            // Spawn the player near a teammate
                            get_respawn_pos(query.world, &random_mate).as_vec3()
                        } else {
                            // There are no other teammates, so spawn the player in a random location
                            query.world.get::<&AsyncRuntime>(|runtime| {
                                query.world.get::<&mut Blocks>(|blocks| {
                                    find_spawn_position(blocks, runtime, &avoid_blocks())
                                })
                            })
                        };

                        client.set::<PendingTeleportation>(PendingTeleportation::new(respawn_pos));
                    });

                    Ok(())
                },
            ));
        });
    }
}

fn get_respawn_pos(world: &World, base_pos: &Position) -> DVec3 {
    let mut position = base_pos.as_dvec3();
    world.get::<&mut Blocks>(|blocks| {
        for x in base_pos.as_i16vec3().x - 15..base_pos.as_i16vec3().x + 15 {
            for y in base_pos.as_i16vec3().y - 15..base_pos.as_i16vec3().y + 15 {
                for z in base_pos.as_i16vec3().z - 15..base_pos.as_i16vec3().z + 15 {
                    let pos = IVec3::new(i32::from(x), i32::from(y), i32::from(z));
                    if let Some(state) = blocks.get_block(pos) {
                        if !is_valid_spawn_block(pos, state, blocks, &avoid_blocks()) {
                            continue;
                        }

                        let block_above1 = blocks.get_block(pos.with_y(pos.y + 1));
                        let block_above2 = blocks.get_block(pos.with_y(pos.y + 2));

                        if let Some(block_above1) = block_above1
                            && let Some(block_above2) = block_above2
                            && block_above1.to_kind() == BlockKind::Air
                            && block_above2.to_kind() == BlockKind::Air
                        {
                            position = pos.with_y(pos.y + 1).as_dvec3();
                            return;
                        }
                    }
                }
            }
        }
    });
    position
}
// From minecraft source
fn get_damage_left(damage: f32, armor: f32, armor_toughness: f32) -> f32 {
    let f: f32 = 2.0 + armor_toughness / 4.0;
    let g: f32 = (armor - damage / f).clamp(armor * 0.2, 20.0);
    damage * (1.0 - g / 25.0)
}

fn get_inflicted_damage(damage: f32, protection: f32) -> f32 {
    let f: f32 = protection.clamp(0.0, 20.0);
    damage * (1.0 - f / 25.0)
}

const fn calculate_damage(item: &ItemStack) -> f32 {
    match item.item {
        ItemKind::WoodenSword | ItemKind::GoldenSword => 4.0,
        ItemKind::StoneSword => 5.0,
        ItemKind::IronSword => 6.0,
        ItemKind::DiamondSword => 7.0,
        ItemKind::NetheriteSword => 8.0,
        ItemKind::WoodenPickaxe => 2.0,
        _ => 1.0,
    }
}

const fn calculate_armor(item: &ItemStack) -> f32 {
    match item.item {
        ItemKind::LeatherHelmet
        | ItemKind::LeatherBoots
        | ItemKind::GoldenHelmet
        | ItemKind::GoldenBoots
        | ItemKind::ChainmailHelmet
        | ItemKind::ChainmailBoots => 1.0,
        ItemKind::LeatherLeggings
        | ItemKind::GoldenLeggings
        | ItemKind::IronHelmet
        | ItemKind::IronBoots => 2.0,
        ItemKind::LeatherChestplate
        | ItemKind::DiamondHelmet
        | ItemKind::DiamondBoots
        | ItemKind::NetheriteHelmet
        | ItemKind::NetheriteBoots => 3.0,
        ItemKind::ChainmailLeggings => 4.0,
        ItemKind::IronLeggings | ItemKind::GoldenChestplate | ItemKind::ChainmailChestplate => 5.0,
        ItemKind::IronChestplate | ItemKind::DiamondLeggings | ItemKind::NetheriteLeggings => 6.0,
        ItemKind::DiamondChestplate | ItemKind::NetheriteChestplate => 8.0,
        _ => 0.0,
    }
}

const fn calculate_toughness(item: &ItemStack) -> f32 {
    match item.item {
        ItemKind::DiamondHelmet
        | ItemKind::DiamondChestplate
        | ItemKind::DiamondLeggings
        | ItemKind::DiamondBoots => 2.0,

        ItemKind::NetheriteHelmet
        | ItemKind::NetheriteChestplate
        | ItemKind::NetheriteLeggings
        | ItemKind::NetheriteBoots => 3.0,
        _ => 0.0,
    }
}

// TODO: split this up into separate functions
fn calculate_stats(inventory: &PlayerInventory, critical_hit: bool) -> CombatStats {
    let hand = inventory.get_cursor();
    let multiplier = if critical_hit { 1.5 } else { 1.0 };
    let damage = calculate_damage(&hand.stack) * multiplier;
    let armor = calculate_armor(&inventory.get_helmet().stack)
        + calculate_armor(&inventory.get_chestplate().stack)
        + calculate_armor(&inventory.get_leggings().stack)
        + calculate_armor(&inventory.get_boots().stack);

    let armor_toughness = calculate_toughness(&inventory.get_helmet().stack)
        + calculate_toughness(&inventory.get_chestplate().stack)
        + calculate_toughness(&inventory.get_leggings().stack)
        + calculate_toughness(&inventory.get_boots().stack);

    CombatStats {
        armor,
        armor_toughness,
        damage,
        // TODO
        protection: 0.0,
    }
}

fn can_critical_hit(player: EntityView<'_>) -> bool {
    player.get::<(&(Prev, Position), &Position)>(|(prev_position, position)| {
        // TODO: Do not allow critical hits if the player is on a ladder, vine, or water. None of
        // these special blocks are currently on the map.
        let position_delta_y = position.y - prev_position.y;
        position_delta_y < 0.0
    })
}
