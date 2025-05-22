use std::borrow::Cow;

use compact_str::format_compact;
use flecs_ecs::{
    core::{
        Builder, ComponentOrPairId, EntityView, EntityViewGet, QueryAPI, QueryBuilderImpl,
        SystemAPI, TableIter, TermBuilderImpl, World, WorldGet, flecs, id,
    },
    macros::{Component, system},
    prelude::Module,
};
use glam::IVec3;
use hyperion::{
    BlockKind, Prev,
    net::{
        Compose, ConnectionId, agnostic,
        packets::{BossBarAction, BossBarS2c},
    },
    runtime::AsyncRuntime,
    simulation::{
        PacketState, PendingTeleportation, Player, Position, Velocity, Xp, Yaw,
        blocks::Blocks,
        event::{AttackEntity, ClientStatusCommand, ClientStatusEvent},
        handlers::PacketSwitchQuery,
        metadata::{entity::Pose, living_entity::Health},
        packet::HandlerRegistry,
    },
    storage::EventQueue,
    uuid::Uuid,
    valence_protocol::{
        ItemKind, ItemStack, Particle, VarInt, ident,
        math::{DVec3, Vec3},
        nbt,
        packets::play::{
            self,
            boss_bar_s2c::{BossBarColor, BossBarDivision, BossBarFlags},
            entity_attributes_s2c::AttributeProperty,
        },
        text::IntoText,
    },
};
use hyperion_inventory::{Inventory, PlayerInventory};
use hyperion_rank_tree::Team;
use hyperion_utils::LifetimeHandle;
use tracing::{event, info_span};

use super::spawn::{avoid_blocks, find_spawn_position, is_valid_spawn_block};

#[derive(Component)]
pub struct AttackModule;

#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct ImmuneUntil {
    tick: i64,
}

#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct Armor {
    pub armor: f32,
}

// Used as a component only for commands, does not include armor or weapons
#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct CombatStats {
    pub armor: f32,
    pub armor_toughness: f32,
    pub damage: f32,
    pub protection: f32,
}

#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct KillCount {
    pub kill_count: u32,
}

#[allow(clippy::cast_possible_truncation)]
impl Module for AttackModule {
    #[allow(clippy::excessive_nesting)]
    #[allow(clippy::cast_sign_loss)]
    fn module(world: &World) {
        world.component::<ImmuneUntil>().meta();
        world.component::<Armor>().meta();
        world.component::<CombatStats>().meta();
        world.component::<KillCount>().meta();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, ImmuneUntil)>()
            .add_trait::<(flecs::With, CombatStats)>()
            .add_trait::<(flecs::With, KillCount)>()
            .add_trait::<(flecs::With, Armor)>();

        let kill_count_uuid = Uuid::new_v4();

        system!(
            "kill_counts",
            world,
            &Compose($),
            &KillCount,
            &ConnectionId,
        )
        .with_enum(PacketState::Play)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each_iter(move |it, _, (compose, kill_count, stream)| {
            const MAX_KILLS: usize = 10;

            let system = it.system();

            let kills = kill_count.kill_count;
            let title = format_compact!("{kills} kills");
            let title = hyperion_text::Text::new(&title);
            let health = (kill_count.kill_count as f32 / MAX_KILLS as f32).min(1.0);

            let pkt = BossBarS2c {
                id: kill_count_uuid,
                action: BossBarAction::Add {
                    title,
                    health,
                    color: BossBarColor::Red,
                    division: BossBarDivision::NoDivision,
                    flags: BossBarFlags::default(),
                },
            };

            compose.unicast(&pkt, *stream, system).unwrap();
        });

        // TODO: This code should be split between melee attacks and bow attacks
        system!("handle_attacks", world, &mut EventQueue<event::AttackEntity>($), &Compose($))

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

                let system = it.system();

                let current_tick = compose.global().tick;

                let world = it.world();

                for event in event_queue.drain() {
                    let origin = world.entity_from_id(event.origin);
                    let target = world.entity_from_id(event.target);

                    let damage = damage_from(origin);
                }

                /*
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

                                    compose.unicast(&pkt_hurt, *target_connection, system).unwrap();

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
                                        compose.broadcast(&particle_pkt, system).exclude(*origin_connection).send().unwrap();
                                    }

                                    if health.is_dead() {
                                        let attacker_name = origin.name();
                                        // Even if enable_respawn_screen is false, the client needs this to send ClientCommandC2s and initiate its respawn
                                        let pkt_death_screen = play::DeathMessageS2c {
                                            player_id: VarInt(target.minecraft_id()),
                                            message: format!("You were killed by {attacker_name}").into_cow_text()
                                        };
                                        compose.unicast(&pkt_death_screen, *target_connection, system).unwrap();
                                    }
                                    compose.broadcast(&sound, system).send().unwrap();
                                    compose.broadcast(&pkt_damage_event, system).send().unwrap();

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
                                                    key: ident!("minecraft:generic.armor").into(),
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
                                        compose.broadcast(&pkt, system).send().unwrap();
                                        compose.broadcast(&particle_pkt, system).send().unwrap();
                                        compose.broadcast(&particle_pkt2, system).send().unwrap();
                                        compose.broadcast(&pkt_entity_status, system).send().unwrap();
                                        compose.broadcast(&pkt_remove_entities, system).send().unwrap();

                                        // Create NBT for enchantment protection level 1
                                        let mut protection_nbt = nbt::Compound::new();
                                        let mut enchantments = vec![];

                                        let mut protection_enchantment = nbt::Compound::new();
                                        protection_enchantment.insert("id", nbt::Value::String("minecraft:protection".into()));
                                        protection_enchantment.insert("lvl", nbt::Value::Short(1));
                                        enchantments.push(protection_enchantment);
                                        protection_nbt.insert(
                                            "Enchantments",
                                            nbt::Value::List(nbt::list::List::Compound(enchantments)),
                                        );
                                        // Apply upgrades based on the level
                                        match kill_count.kill_count {
                                            0 => {}
                                            1 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::WoodenSword, 1, None)).unwrap(),
                                            2 => inventory
                                                .set_boots(ItemStack::new(ItemKind::LeatherBoots, 1, None)),
                                            3 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::LeatherLeggings, 1, None)),
                                            4 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::LeatherChestplate, 1, None)),
                                            5 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::LeatherHelmet, 1, None)),
                                            6 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::StoneSword, 1, None)).unwrap(),
                                            7 => inventory
                                                .set_boots(ItemStack::new(ItemKind::ChainmailBoots, 1, None)),
                                            8 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::ChainmailLeggings, 1, None)),
                                            9 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::ChainmailChestplate, 1, None)),
                                            10 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::ChainmailHelmet, 1, None)),
                                            11 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::IronSword, 1, None)).unwrap(),
                                            12 => inventory
                                                .set_boots(ItemStack::new(ItemKind::IronBoots, 1, None)),
                                            13 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::IronLeggings, 1, None)),
                                            14 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::IronChestplate, 1, None)),
                                            15 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::IronHelmet, 1, None)),
                                            16 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::DiamondSword, 1, None)).unwrap(),
                                            17 => inventory
                                                .set_boots(ItemStack::new(ItemKind::DiamondBoots, 1, None)),
                                            18 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::DiamondLeggings, 1, None)),
                                            19 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::DiamondChestplate, 1, None)),
                                            20 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::DiamondHelmet, 1, None)),
                                            21 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::NetheriteSword, 1, None)).unwrap(),
                                            22 => inventory
                                                .set_boots(ItemStack::new(ItemKind::NetheriteBoots, 1, None)),
                                            23 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::NetheriteLeggings, 1, None)),
                                            24 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::NetheriteChestplate, 1, None)),
                                            25 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::NetheriteHelmet, 1, None)),
                                            26 => {
                                                // Reset armor and start again with Protection I
                                                inventory.set_boots(ItemStack::new(
                                                    ItemKind::LeatherBoots,
                                                    1,
                                                    Some(protection_nbt.clone()),
                                                ));
                                                inventory.set_leggings(ItemStack::new(
                                                    ItemKind::LeatherLeggings,
                                                    1,
                                                    Some(protection_nbt.clone()),
                                                ));
                                                inventory.set_chestplate(ItemStack::new(
                                                    ItemKind::LeatherChestplate,
                                                    1,
                                                    Some(protection_nbt.clone()),
                                                ));
                                                inventory.set_helmet(ItemStack::new(
                                                    ItemKind::LeatherHelmet,
                                                    1,
                                                    Some(protection_nbt.clone()),
                                                ));
                                            }
                                            _ => {
                                                // Continue upgrading with Protection I after reset
                                                let level = (kill_count.kill_count - 26) % 24;
                                                match level {
                                                    1 => inventory.set_boots(ItemStack::new(
                                                        ItemKind::ChainmailBoots,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    2 => inventory.set_leggings(ItemStack::new(
                                                        ItemKind::ChainmailLeggings,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    3 => inventory.set_chestplate(ItemStack::new(
                                                        ItemKind::ChainmailChestplate,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    4 => inventory.set_helmet(ItemStack::new(
                                                        ItemKind::ChainmailHelmet,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    5 => inventory.set_boots(ItemStack::new(
                                                        ItemKind::IronBoots,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    6 => inventory.set_leggings(ItemStack::new(
                                                        ItemKind::IronLeggings,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    7 => inventory.set_chestplate(ItemStack::new(
                                                        ItemKind::IronChestplate,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    8 => inventory.set_helmet(ItemStack::new(
                                                        ItemKind::IronHelmet,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    9 => inventory.set_boots(ItemStack::new(
                                                        ItemKind::DiamondBoots,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    10 => inventory.set_leggings(ItemStack::new(
                                                        ItemKind::DiamondLeggings,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    11 => inventory.set_chestplate(ItemStack::new(
                                                        ItemKind::DiamondChestplate,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    12 => inventory.set_helmet(ItemStack::new(
                                                        ItemKind::DiamondHelmet,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    13 => inventory.set_boots(ItemStack::new(
                                                        ItemKind::NetheriteBoots,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    14 => inventory.set_leggings(ItemStack::new(
                                                        ItemKind::NetheriteLeggings,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    15 => inventory.set_chestplate(ItemStack::new(
                                                        ItemKind::NetheriteChestplate,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    16 => inventory.set_helmet(ItemStack::new(
                                                        ItemKind::NetheriteHelmet,
                                                        1,
                                                        Some(protection_nbt.clone()),
                                                    )),
                                                    _ => {} // No upgrade for other levels
                                                }
                                            }
                                        }
                                        // player died, increment kill count
                                        kill_count.kill_count += 1;

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
        */
                },
            );

        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |client_status: &ClientStatusEvent,
                 _: &dyn LifetimeHandle<'_>,
                 query: &mut PacketSwitchQuery<'_>| {
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

fn is_falling(entity: EntityView<'_>) -> bool {
    entity.get::<(&Position, &(Prev, Position))>(|(position, prev_position)| {
        position.y - prev_position.y < 0.0
    })
}

fn can_critical_hit(entity: EntityView<'_>) -> bool {
    is_falling(entity)
}
// // pipe
//
// fn damage_from(entity: EntityView<'_>) -> eyre::Result<f32> {
//     let can_critical_hit = can_critical_hit(entity);
//
//     entity.get::<&CombatStats>(|stats| {
//         stats.damage
//     })
// }
