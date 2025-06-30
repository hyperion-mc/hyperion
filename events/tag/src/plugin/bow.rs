use std::time::{Duration, SystemTime};

use bevy::prelude::*;
use hyperion::{
    ItemKind, ItemStack,
    glam::Vec3,
    net::Compose,
    simulation::{
        Owner, Pitch, Position, SpawnEvent, Uuid, Velocity, Yaw,
        entity_kind::EntityKind,
        event, get_direction_from_rotation,
        metadata::living_entity::{ArrowsInEntity, HandStates},
        packet_state,
    },
};
use hyperion_inventory::PlayerInventory;
use hyperion_utils::EntityExt;
use tracing::{debug, error};
use valence_protocol::{VarInt, ident, packets::play};

#[derive(Component)]
pub struct LastFireTime {
    pub time: SystemTime,
}

impl LastFireTime {
    pub fn now() -> Self {
        Self {
            time: SystemTime::now(),
        }
    }

    // if above 150ms, can fire
    pub fn can_fire(&self) -> bool {
        let elapsed = self.time.elapsed().unwrap_or(Duration::ZERO);
        elapsed.as_millis() > 150
    }
}

#[derive(Component, Default)]
pub struct BowCharging {
    pub start_time: Option<SystemTime>,
}

impl BowCharging {
    #[must_use]
    pub fn now() -> Self {
        Self {
            start_time: Some(SystemTime::now()),
        }
    }

    #[must_use]
    pub fn get_charge(&self) -> Option<f32> {
        let elapsed = self.start_time?.elapsed().unwrap_or(Duration::ZERO);
        let secs = elapsed.as_secs_f32();
        // Minecraft bow charge mechanics:
        // - Takes 1.2 second to fully charge
        // - Minimum charge is 0.000001
        // - Maximum charge is 1.0
        Some(secs.clamp(0.01, 1.2))
    }

    pub const fn reset(&mut self) {
        self.start_time = None;
    }
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert((LastFireTime::now(), BowCharging::default()));
}

fn handle_bow_use(
    mut events: EventReader<'_, '_, event::ItemInteract>,
    query: Query<'_, '_, &PlayerInventory>,
    mut commands: Commands<'_, '_>,
) {
    for event in events.read() {
        let inventory = match query.get(event.entity) {
            Ok(inventory) => inventory,
            Err(e) => {
                error!("failed to handle bow use: query failed: {e}");
                continue;
            }
        };

        let cursor = inventory.get_cursor();
        if cursor.stack.item != ItemKind::Bow {
            return;
        }

        commands
            .entity(event.entity)
            .insert((BowCharging::now(), HandStates::new(1)));
    }
}

fn handle_bow_release(
    mut events: EventReader<'_, '_, event::ReleaseUseItem>,
    mut query: Query<
        '_,
        '_,
        (
            &mut LastFireTime,
            &mut PlayerInventory,
            &Position,
            &Yaw,
            &Pitch,
            &mut BowCharging,
        ),
    >,
    mut spawn_writer: EventWriter<'_, SpawnEvent>,
    mut commands: Commands<'_, '_>,
) {
    for event in events.read() {
        let (mut last_fire_time, mut inventory, position, yaw, pitch, mut bow_charging) =
            match query.get_mut(event.from) {
                Ok(data) => data,
                Err(e) => {
                    error!("failed to handle bow release: query failed: {e}");
                    continue;
                }
            };

        if inventory.get_cursor().stack.item != ItemKind::Bow {
            continue;
        }

        // Check the cooldown
        if !last_fire_time.can_fire() {
            continue;
        }

        // Check if the player has enough arrows in their inventory
        let items: Vec<(u16, &ItemStack)> = inventory.items().collect();
        let mut has_arrow = false;
        for (slot, item) in items {
            if item.item == ItemKind::Arrow && item.count >= 1 {
                let count = item.count - 1;
                if count == 0 {
                    inventory.set(slot, ItemStack::EMPTY).unwrap();
                } else {
                    let stack = ItemStack::new(item.item, count, item.nbt.clone());
                    inventory.set(slot, stack).unwrap();
                }
                has_arrow = true;
                break;
            }
        }

        if !has_arrow {
            continue;
        }

        // Update the last fire time
        *last_fire_time = LastFireTime::now();

        // Get how charged the bow is
        let Some(charge) = bow_charging.get_charge() else {
            error!("player attempted to release a non-charged bow");
            continue;
        };

        bow_charging.reset();

        debug!(
            "Player {:?} fired an arrow with charge {}",
            event.from, charge
        );

        // Calculate the direction vector from the player's rotation
        let direction = get_direction_from_rotation(**yaw, **pitch);
        // Calculate the velocity of the arrow based on the charge (3.0 is max velocity)
        let velocity = direction * (charge * 3.0);

        let spawn_pos = Vec3::new(position.x, position.y + 1.62, position.z) + direction * 0.5;

        debug!("Arrow spawn position: {:?}", spawn_pos);

        let id = commands
            .spawn((
                Uuid::new_v4(),
                Position::new(spawn_pos.x, spawn_pos.y, spawn_pos.z),
                Velocity::new(velocity.x, velocity.y, velocity.z),
                Pitch::new(**pitch),
                Yaw::new(**yaw),
                Owner::new(event.from),
                EntityKind::Arrow,
            ))
            .id();

        spawn_writer.write(SpawnEvent(id));
    }
}

fn arrow_entity_hit(
    mut events: EventReader<'_, '_, event::ProjectileEntityEvent>,
    compose: Res<'_, Compose>,
    arrow_query: Query<'_, '_, (&Velocity, &Owner)>,
    mut player_query: Query<'_, '_, (&Position, &mut ArrowsInEntity)>,
    mut commands: Commands<'_, '_>,
    mut writer: EventWriter<'_, event::AttackEntity>,
) {
    for event in events.read() {
        let (velocity, owner) = match arrow_query.get(event.projectile) {
            Ok(data) => data,
            Err(e) => {
                error!("arrow entity hit failed: arrow query failed: {e}");
                continue;
            }
        };

        let (position, mut arrows) = match player_query.get_mut(event.client) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("arrow entity hit failed: player query failed: {e}");
                continue;
            }
        };

        let damage = velocity.0.length() * 2.0;
        let chunk_pos = position.to_chunk();

        if damage == 0.0 && owner.entity == event.client {
            continue;
        }

        arrows.0 += 1;

        let packet = play::EntitiesDestroyS2c {
            entity_ids: vec![VarInt(event.projectile.minecraft_id())].into(),
        };

        compose.broadcast_local(&packet, chunk_pos).send().unwrap();

        commands.entity(event.projectile).despawn();

        writer.write(event::AttackEntity {
            origin: owner.entity,
            target: event.client,
            direction: velocity.0.normalize(),
            damage,
            sound: ident!("entity.arrow.hit_player"),
            particles: None,
        });
    }
}

fn arrow_block_hit(
    mut events: EventReader<'_, '_, event::ProjectileBlockEvent>,
    mut query: Query<'_, '_, (&mut Position, &mut Velocity)>,
) {
    for event in events.read() {
        let (mut position, mut velocity) = match query.get_mut(event.projectile) {
            Ok(data) => data,
            Err(e) => {
                error!("arrow block hit failed: query failed: {e}");
                continue;
            }
        };

        velocity.0 = Vec3::ZERO;
        **position = event.collision.point;
    }
}

pub struct BowPlugin;

impl Plugin for BowPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_player);
        app.add_systems(
            FixedUpdate,
            (
                (handle_bow_use, handle_bow_release).chain(),
                arrow_entity_hit,
                arrow_block_hit,
            ),
        );
    }
}
