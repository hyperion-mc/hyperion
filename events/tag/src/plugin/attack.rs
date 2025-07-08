use std::borrow::Cow;

use bevy::prelude::*;
use compact_str::format_compact;
use derive_more::with_trait::Add;
use glam::IVec3;
use hyperion::{
    BlockKind, ingress,
    net::{
        Compose, ConnectionId, DataBundle, agnostic,
        packets::{BossBarAction, BossBarS2c},
    },
    runtime::AsyncRuntime,
    simulation::{
        Flight, FlyingSpeed, Pitch, Position, TeleportEvent, Velocity, Xp, Yaw,
        blocks::Blocks,
        event,
        metadata::{entity::Pose, living_entity::Health},
        packet::play,
        packet_state,
    },
    uuid::Uuid,
};
use hyperion_inventory::PlayerInventory;
use hyperion_rank_tree::Team;
use hyperion_utils::{EntityExt, Prev};
use tracing::error;
use valence_protocol::{
    BlockPos, ByteAngle, GameMode, GlobalPos, ItemKind, ItemStack, Particle, VarInt,
    game_mode::OptGameMode,
    ident,
    math::{DVec3, Vec3},
    packets::play::{
        DamageTiltS2c, DeathMessageS2c, EntityDamageS2c, ExperienceBarUpdateS2c, GameMessageS2c,
        ParticleS2c, PlayerRespawnS2c, PlayerSpawnS2c,
        boss_bar_s2c::{BossBarColor, BossBarDivision, BossBarFlags},
        client_status_c2s::ClientStatusC2s,
        player_abilities_s2c::{PlayerAbilitiesFlags, PlayerAbilitiesS2c},
        player_interact_entity_c2s::EntityInteraction,
    },
    text::IntoText,
};

use super::spawn::{avoid_blocks, find_spawn_position, is_valid_spawn_block};

pub struct AttackPlugin;

#[derive(Component, Default, Copy, Clone, Debug)]
pub struct ImmuneUntil {
    tick: i64,
}

// Used as a component only for commands, does not include armor or weapons
#[derive(Component, Default, Copy, Clone, Debug, Add)]
pub struct CombatStats {
    pub armor: f32,
    pub armor_toughness: f32,
    pub damage: f32,
    pub protection: f32,
}

#[derive(Component, Default, Copy, Clone, Debug)]
pub struct KillCount {
    pub kill_count: u32,
}

#[derive(Resource)]
struct KillCountUuid(Uuid);

/// Checks if the entity is immune to attacks and updates the immunity timer if it is
///
/// Returns true if the entity is immune, false otherwise
const fn check_and_update_immunity(tick: i64, immune_until: &mut ImmuneUntil) -> bool {
    const IMMUNE_TICK_DURATION: i64 = 10;
    if immune_until.tick > tick {
        return true;
    }

    immune_until.tick = tick + IMMUNE_TICK_DURATION;

    false
}

fn is_critical_hit(prev_position: Prev<Position>, position: Position) -> bool {
    // TODO: Do not allow critical hits if the player is on a ladder, vine, or water. None of
    // these special blocks are currently on the map.
    let position_delta_y = position.y - prev_position.y;
    position_delta_y < 0.0
}

fn total_combat_stats(
    critical_hit: bool,
    inventory: &PlayerInventory,
    stats: CombatStats,
) -> CombatStats {
    let inventory_combat_stats = calculate_stats(inventory, critical_hit);
    stats + inventory_combat_stats
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands.entity(trigger.target()).insert((
        ImmuneUntil::default(),
        CombatStats::default(),
        KillCount::default(),
    ));
}

fn handle_melee_attacks(
    mut packets: EventReader<'_, '_, play::PlayerInteractEntity>,
    origin_query: Query<'_, '_, (&Position, &PlayerInventory, &CombatStats)>,
    target_query: Query<'_, '_, (&Prev<Position>, &Position)>,
    mut world_and_writer: ParamSet<'_, '_, (&World, EventWriter<'_, event::AttackEntity>)>,
) {
    for packet in packets.read() {
        if packet.interact != EntityInteraction::Attack {
            continue;
        }

        // Player who is attacking the target
        let origin = packet.sender();

        // Player who is being attacked by the attacker
        let target = match Entity::from_minecraft_id(packet.entity_id.0, world_and_writer.p0()) {
            Ok(target) => target,
            Err(e) => {
                error!("handle melee attack failed: target id is invalid: {e}");
                continue;
            }
        };

        let (&origin_pos, origin_inventory, &origin_stats) = match origin_query.get(origin) {
            Ok(data) => data,
            Err(e) => {
                error!("handle melee attack failed: query failed: {e}");
                continue;
            }
        };

        let (&target_prev_pos, &target_pos) = match target_query.get(target) {
            Ok(data) => data,
            Err(e) => {
                error!("handle melee attack failed: query failed: {e}");
                continue;
            }
        };

        let is_critical_hit = is_critical_hit(target_prev_pos, target_pos);
        let combat_stats = total_combat_stats(is_critical_hit, origin_inventory, origin_stats);

        let damage_after_armor =
            get_damage_left(1.0, combat_stats.armor, combat_stats.armor_toughness);
        let damage_after_protection =
            get_inflicted_damage(damage_after_armor, combat_stats.protection);

        world_and_writer.p1().write(event::AttackEntity {
            origin,
            target,
            direction: (target_pos - origin_pos).normalize(),
            damage: damage_after_protection,
            sound: if is_critical_hit {
                ident!("minecraft:entity.player.attack.crit")
            } else {
                ident!("minecraft:entity.player.attack.knockback")
            },
            particles: is_critical_hit.then(|| ParticleS2c {
                particle: Cow::Owned(Particle::Crit),
                long_distance: true,
                position: target_pos.as_dvec3() + DVec3::new(0.0, 1.0, 0.0),
                max_speed: 0.5,
                count: 100,
                offset: Vec3::new(0.5, 0.5, 0.5),
            }),
        });
    }
}

fn handle_attacks(
    mut events: EventReader<'_, '_, event::AttackEntity>,
    compose: Res<'_, Compose>,
    mut origin_query: Query<'_, '_, (&Team, &Name, &ConnectionId, &mut KillCount)>,
    mut target_query: Query<
        '_,
        '_,
        (
            &Team,
            &Position,
            &Yaw,
            &ConnectionId,
            &mut ImmuneUntil,
            &mut Health,
            &mut Velocity,
        ),
    >,
) {
    let current_tick = compose.global().tick;

    for event in events.read() {
        if event.damage <= 0.0 {
            continue;
        }

        let (origin_team, origin_name, &origin_connection, mut origin_kill_count) =
            match origin_query.get_mut(event.origin) {
                Ok(data) => data,
                Err(e) => {
                    error!("handle melee attack failed: query failed: {e}");
                    continue;
                }
            };

        let (
            target_team,
            &target_pos,
            &target_yaw,
            &target_connection,
            mut target_immune_until,
            mut target_health,
            mut target_velocity,
        ) = match target_query.get_mut(event.target) {
            Ok(data) => data,
            Err(e) => {
                error!("handle melee attack failed: query failed: {e}");
                continue;
            }
        };

        if origin_team == target_team {
            let msg = "§cCannot attack teammates";
            let pkt_msg = GameMessageS2c {
                chat: msg.into_cow_text(),
                overlay: false,
            };

            compose.unicast(&pkt_msg, origin_connection).unwrap();

            continue;
        }

        if check_and_update_immunity(current_tick, &mut target_immune_until) {
            // no damage; the target is immune
            continue;
        }

        // Broadcast sound
        let sound = agnostic::sound(event.sound.clone(), *target_pos)
            .volume(1.)
            .pitch(1.)
            .seed(fastrand::i64(..))
            .build();

        compose.broadcast(&sound).send().unwrap();

        // Broadcast particles
        if let Some(particles) = &event.particles {
            compose
                .broadcast(particles)
                .exclude(origin_connection)
                .send()
                .unwrap();
        }

        let delta_x: f64 = f64::from(event.direction.x);
        let delta_z: f64 = f64::from(event.direction.z);

        // Seems that MC generates a random delta if the damage source is too close to the target
        // let's ignore that for now
        #[expect(clippy::cast_possible_truncation)]
        let pkt_hurt = DamageTiltS2c {
            entity_id: VarInt(event.target.minecraft_id()),
            yaw: delta_z
                .atan2(delta_x)
                .mul_add(57.295_776_367_187_5_f64, -f64::from(*target_yaw)) as f32,
        };

        compose.unicast(&pkt_hurt, target_connection).unwrap();

        target_health.damage(event.damage);

        if target_health.is_dead() {
            // Even if enable_respawn_screen is false, the client needs this to send ClientCommandC2s and initiate its respawn
            let pkt_death_screen = DeathMessageS2c {
                player_id: VarInt(event.target.minecraft_id()),
                message: format!("You were killed by {origin_name}").into_cow_text(),
            };
            compose
                .unicast(&pkt_death_screen, target_connection)
                .unwrap();

            origin_kill_count.kill_count += 1;
        } else {
            // Calculate velocity change based on attack direction
            let knockback_xz = 8.0;
            let knockback_y = 6.432;

            let new_vel = Vec3::new(
                event.direction.x * knockback_xz / 20.0,
                knockback_y / 20.0,
                event.direction.z * knockback_xz / 20.0,
            );

            target_velocity.0 += new_vel;
        }

        // EntityDamageS2c: display red outline when taking damage (play arrow hit sound?)
        let pkt_damage_event = EntityDamageS2c {
            entity_id: VarInt(event.target.minecraft_id()),
            source_cause_id: VarInt(event.origin.minecraft_id() + 1), // this is an OptVarint
            source_direct_id: VarInt(event.origin.minecraft_id() + 1), /* if hit by a projectile, it should be the projectile's entity id */
            source_type_id: VarInt(31),                                // 31 = player_attack
            source_pos: None,
        };
        compose.broadcast(&pkt_damage_event).send().unwrap();
    }
}

fn handle_respawn(
    mut packets: EventReader<'_, '_, play::ClientStatus>,
    mut query: Query<
        '_,
        '_,
        (
            &hyperion::simulation::Uuid,
            &Xp,
            &Flight,
            &FlyingSpeed,
            &Team,
            &Position,
            &Yaw,
            &Pitch,
            &mut Health,
            &mut Pose,
        ),
    >,
    candidates_query: Query<'_, '_, (Entity, &Position, &Team)>,
    mut blocks: ResMut<'_, Blocks>,
    runtime: Res<'_, AsyncRuntime>,
    compose: Res<'_, Compose>,
    mut teleport_writer: EventWriter<'_, TeleportEvent>,
) {
    for packet in packets.read() {
        if !matches!(**packet, ClientStatusC2s::PerformRespawn) {
            continue;
        }

        let (
            uuid,
            xp,
            flight,
            flying_speed,
            team,
            last_death_location,
            yaw,
            pitch,
            mut health,
            mut pose,
        ) = match query.get_mut(packet.sender()) {
            Ok(team) => team,
            Err(e) => {
                error!("handle respawn failed: query failed: {e}");
                continue;
            }
        };

        if !health.is_dead() {
            continue;
        }

        health.heal(20.);

        *pose = Pose::Standing;

        let pos_vec = candidates_query
            .iter()
            .filter(|(candidate_entity, _, candidate_team)| {
                team == *candidate_team && *candidate_entity != packet.sender()
            })
            .map(|(_, &pos, _)| pos)
            .collect::<Vec<_>>();

        let respawn_pos = if let Some(random_mate) = fastrand::choice(pos_vec) {
            // Spawn the player near a teammate
            get_respawn_pos(&blocks, &random_mate).as_vec3()
        } else {
            // There are no other teammates, so spawn the player in a random location
            find_spawn_position(&mut blocks, &runtime, &avoid_blocks())
        };

        teleport_writer.write(TeleportEvent {
            player: packet.sender(),
            destination: respawn_pos,
            reset_velocity: true,
        });

        let pkt_respawn = PlayerRespawnS2c {
            dimension_type_name: ident!("minecraft:overworld"),
            dimension_name: ident!("minecraft:overworld"),
            hashed_seed: 0,
            game_mode: GameMode::Survival,
            previous_game_mode: OptGameMode::default(),
            is_debug: false,
            is_flat: false,
            copy_metadata: false,
            last_death_location: Option::from(GlobalPos {
                dimension_name: ident!("minecraft:overworld"),
                position: BlockPos::from(last_death_location.as_dvec3()),
            }),
            portal_cooldown: VarInt::default(),
        };

        let pkt_xp = ExperienceBarUpdateS2c {
            bar: xp.get_visual().prop,
            level: VarInt(i32::from(xp.get_visual().level)),
            total_xp: VarInt::default(),
        };

        let pkt_abilities = PlayerAbilitiesS2c {
            flags: PlayerAbilitiesFlags::default()
                .with_flying(flight.is_flying)
                .with_allow_flying(flight.allow),
            flying_speed: flying_speed.speed,
            fov_modifier: 0.0,
        };

        let mut bundle = DataBundle::new(&compose);
        bundle.add_packet(&pkt_respawn).unwrap();
        bundle.add_packet(&pkt_xp).unwrap();
        bundle.add_packet(&pkt_abilities).unwrap();
        bundle.unicast(packet.connection_id()).unwrap();

        let pkt_add_player = PlayerSpawnS2c {
            entity_id: VarInt(packet.minecraft_id()),
            player_uuid: uuid.0,
            position: respawn_pos.as_dvec3(),
            yaw: ByteAngle::from_degrees(**yaw),
            pitch: ByteAngle::from_degrees(**pitch),
        };

        compose
            .broadcast(&pkt_add_player)
            .exclude(packet.connection_id())
            .send()
            .unwrap();
    }
}

fn update_kill_counts(
    query: Query<'_, '_, (&KillCount, &ConnectionId), Changed<KillCount>>,
    kill_count_uuid: Res<'_, KillCountUuid>,
    compose: Res<'_, Compose>,
) {
    const MAX_KILLS: usize = 10;

    query.par_iter().for_each(|(kill_count, &connection_id)| {
        let kills = kill_count.kill_count;
        let title = format_compact!("{kills} kills");
        let title = hyperion_text::Text::new(&title);
        let health = (kill_count.kill_count as f32 / MAX_KILLS as f32).min(1.0);

        let pkt = BossBarS2c {
            id: kill_count_uuid.0,
            action: BossBarAction::Add {
                title,
                health,
                color: BossBarColor::Red,
                division: BossBarDivision::NoDivision,
                flags: BossBarFlags::default(),
            },
        };

        compose.unicast(&pkt, connection_id).unwrap();
    });
}

#[allow(clippy::cast_possible_truncation)]
impl Plugin for AttackPlugin {
    #[allow(clippy::excessive_nesting)]
    #[allow(clippy::cast_sign_loss)]
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_player);
        app.insert_resource(KillCountUuid(Uuid::new_v4()));
        app.add_systems(
            FixedUpdate,
            (
                (
                    (handle_melee_attacks, handle_attacks).chain(),
                    handle_respawn,
                )
                    .after(ingress::decode::play),
                update_kill_counts,
            ),
        );
    }
}

fn get_respawn_pos(blocks: &Blocks, base_pos: &Position) -> DVec3 {
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
                        return pos.with_y(pos.y + 1).as_dvec3();
                    }
                }
            }
        }
    }
    base_pos.as_dvec3()
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
