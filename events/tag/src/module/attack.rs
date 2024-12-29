use std::borrow::Cow;

use compact_str::format_compact;
use flecs_ecs::{
    core::{
        Builder, EntityViewGet, QueryAPI, QueryBuilderImpl, SystemAPI, TableIter, TermBuilderImpl,
        World, WorldGet, flecs,
    },
    macros::{Component, system},
    prelude::Module,
};
use glam::IVec3;
use hyperion::{
    net::{
        Compose, ConnectionId, agnostic,
        packets::{BossBarAction, BossBarS2c},
    },
    simulation::{
        PacketState, Pitch, Player, Position, Velocity, Xp, Yaw,
        blocks::Blocks,
        event::{self, ClientStatusCommand},
        metadata::{entity::Pose, living_entity::Health},
    },
    storage::{EventQueue, GlobalEventHandlers},
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
use hyperion_inventory::PlayerInventory;
use hyperion_rank_tree::Team;
use hyperion_utils::EntityExt;
use tracing::info_span;
use valence_protocol::packets::play::player_position_look_s2c::PlayerPositionLookFlags;

use super::spawn::{avoid_blocks, is_valid_spawn_block};

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
        .multi_threaded()
        .kind::<flecs::pipeline::OnUpdate>()
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

        system!("handle_attacks", world, &mut EventQueue<event::AttackEntity>($), &Compose($))
            .multi_threaded()
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
                        let target = world.entity_from_id(event.target);
                        let origin = world.entity_from_id(event.origin);
                        origin.get::<(&ConnectionId, &Position, &mut KillCount, &mut PlayerInventory, &mut Armor, &CombatStats, &PlayerInventory, &Team, &mut Xp)>(|(origin_connection, origin_pos, kill_count, inventory, origin_armor, from_stats, from_inventory, origin_team, origin_xp)| {
                            let damage = from_stats.damage + calculate_stats(from_inventory).damage;
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
                                &mut Xp
                            )>(
                                |(target_connection, immune_until, health, target_position, target_yaw, stats, target_inventory, target_team, target_pose, target_xp)| {
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

                                        compose.unicast(&pkt_msg, *origin_connection, system).unwrap();
                                        return;
                                    }

                                    let calculated_stats = calculate_stats(target_inventory);
                                    let armor = stats.armor + calculated_stats.armor;
                                    let toughness = stats.armor_toughness + calculated_stats.armor_toughness;
                                    let protection = stats.protection + calculated_stats.protection;

                                    let damage_after_armor = get_damage_left(damage, armor, toughness);
                                    let damage_after_protection = get_inflicted_damage(damage_after_armor, protection);

                                    health.damage(damage_after_protection);

                                    let pkt_health = play::HealthUpdateS2c {
                                        health: health.abs(),
                                        food: VarInt(20),
                                        food_saturation: 5.0
                                    };

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
                                        source_pos: Option::None
                                    };
                                    let sound = agnostic::sound(
                                        ident!("minecraft:entity.player.attack.knockback"),
                                        **target_position,
                                    ).volume(1.)
                                    .pitch(1.)
                                    .seed(fastrand::i64(..))
                                    .build();

                                    compose.unicast(&pkt_hurt, *target_connection, system).unwrap();
                                    compose.unicast(&pkt_health, *target_connection, system).unwrap();

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
                                        target.modified::<Pose>();
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
                                                .set_hotbar(0, ItemStack::new(ItemKind::WoodenSword, 1, None)),
                                            2 => inventory
                                                .set_boots(ItemStack::new(ItemKind::LeatherBoots, 1, None)),
                                            3 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::LeatherLeggings, 1, None)),
                                            4 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::LeatherChestplate, 1, None)),
                                            5 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::LeatherHelmet, 1, None)),
                                            6 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::StoneSword, 1, None)),
                                            7 => inventory
                                                .set_boots(ItemStack::new(ItemKind::ChainmailBoots, 1, None)),
                                            8 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::ChainmailLeggings, 1, None)),
                                            9 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::ChainmailChestplate, 1, None)),
                                            10 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::ChainmailHelmet, 1, None)),
                                            11 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::IronSword, 1, None)),
                                            12 => inventory
                                                .set_boots(ItemStack::new(ItemKind::IronBoots, 1, None)),
                                            13 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::IronLeggings, 1, None)),
                                            14 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::IronChestplate, 1, None)),
                                            15 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::IronHelmet, 1, None)),
                                            16 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::DiamondSword, 1, None)),
                                            17 => inventory
                                                .set_boots(ItemStack::new(ItemKind::DiamondBoots, 1, None)),
                                            18 => inventory
                                                .set_leggings(ItemStack::new(ItemKind::DiamondLeggings, 1, None)),
                                            19 => inventory
                                                .set_chestplate(ItemStack::new(ItemKind::DiamondChestplate, 1, None)),
                                            20 => inventory
                                                .set_helmet(ItemStack::new(ItemKind::DiamondHelmet, 1, None)),
                                            21 => inventory
                                                .set_hotbar(0, ItemStack::new(ItemKind::NetheriteSword, 1, None)),
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

                                    let new_vel = Velocity::new(
                                        dir.x * knockback_xz / 20.0,
                                        knockback_y / 20.0,
                                        dir.z * knockback_xz / 20.0
                                    );

                                    // https://github.com/valence-rs/valence/blob/8f3f84d557dacddd7faddb2ad724185ecee2e482/examples/ctf.rs#L987-L989
                                    let packet = play::EntityVelocityUpdateS2c {
                                        entity_id: VarInt(target.minecraft_id()),
                                        velocity: new_vel.to_packet_units(),
                                    };

                                    compose.broadcast_local(&packet, target_position.to_chunk(), system).send().unwrap();
                                },
                            );
                        });
                    }
                },
            );

        world.get::<&mut GlobalEventHandlers>(|handlers| {
            handlers.client_status.register(|query, client_status| {
                if client_status.status == ClientStatusCommand::RequestStats {
                    return;
                }

                let client = client_status.client.entity_view(query.world);

                client.get::<(&Team, &mut Position, &Yaw, &Pitch, &ConnectionId)>(
                    |(team, position, yaw, pitch, connection)| {
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

                        let random_index = fastrand::usize(..pos_vec.len());

                        if let Some(random_mate) = pos_vec.get(random_index) {
                            let respawn_pos = get_respawn_pos(query.world, random_mate);

                            *position = Position::from(respawn_pos.as_vec3());

                            let pkt_teleport = play::PlayerPositionLookS2c {
                                position: respawn_pos,
                                yaw: **yaw,
                                pitch: **pitch,
                                flags: PlayerPositionLookFlags::default(),
                                teleport_id: VarInt(fastrand::i32(..)),
                            };

                            query
                                .compose
                                .unicast(&pkt_teleport, *connection, query.system)
                                .unwrap();
                        }
                    },
                );
            });
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
                    match blocks.get_block(pos) {
                        Some(state) => {
                            if is_valid_spawn_block(pos, state, blocks, &avoid_blocks()) {
                                position = pos.as_dvec3();
                                return;
                            }
                        }
                        None => continue,
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

fn calculate_stats(inventory: &PlayerInventory) -> CombatStats {
    let hand = inventory.get_cursor();
    let damage = calculate_damage(&hand.stack);
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
