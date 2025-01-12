use flecs_ecs::{
    core::{EntityViewGet, World},
    prelude::*,
};
use hyperion::{
    ItemKind, ItemStack,
    glam::Vec3,
    simulation::{
        Pitch, Position, Spawn, Uuid, Velocity, Yaw, bow::BowCharging, entity_kind::EntityKind,
        event, get_direction_from_rotation, metadata::living_entity::ArrowsInEntity,
    },
    storage::{EventQueue, Events},
};
use hyperion_inventory::PlayerInventory;
use tracing::debug;

#[derive(Component)]
pub struct BowModule;

#[derive(Component)]
pub struct Owner {
    pub entity: Entity,
}

impl Owner {
    pub const fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

impl Module for BowModule {
    fn module(world: &World) {
        world.component::<Owner>();

        system!(
            "handle_bow_release",
            world,
            &mut EventQueue<event::ReleaseUseItem>,
        )
        .term_at(0u32)
        .singleton()
        .kind::<flecs::pipeline::PostUpdate>()
        .each_iter(move |it, _, event_queue| {
            let _system = it.system();
            let world = it.world();

            for event in event_queue.drain() {
                if event.item != ItemKind::Bow {
                    continue;
                }

                let player = world.entity_from_id(event.from);

                #[allow(clippy::excessive_nesting)]
                player.get::<(&mut PlayerInventory, &Position, &Yaw, &Pitch)>(
                    |(inventory, position, yaw, pitch)| {
                        // check if the player has enough arrows in their inventory
                        let items: Vec<(u16, &ItemStack)> = inventory.items().collect();
                        let mut has_arrow = false;
                        for (slot, item) in items {
                            if item.item == ItemKind::Arrow && item.count >= 1 {
                                let count = item.count - 1;
                                if count == 0 {
                                    inventory.set(slot, ItemStack::EMPTY).unwrap();
                                } else {
                                    inventory
                                        .set(
                                            slot,
                                            ItemStack::new(item.item, count, item.nbt.clone()),
                                        )
                                        .unwrap();
                                }
                                has_arrow = true;
                                break;
                            }
                        }

                        if !has_arrow {
                            return;
                        }

                        // Get how charged the bow is
                        let charge = event
                            .from
                            .entity_view(world)
                            .try_get::<&BowCharging>(|charging| {
                                let charge = charging.get_charge();
                                event.from.entity_view(world).remove::<BowCharging>();
                                charge
                            })
                            .unwrap_or(0.0);

                        debug!("charge: {charge}");

                        // Calculate the direction vector from the player's rotation
                        let direction = get_direction_from_rotation(**yaw, **pitch);
                        // Calculate the velocity of the arrow based on the charge (3.0 is max velocity)
                        let velocity = direction * (charge * 3.0);

                        let spawn_pos =
                            Vec3::new(position.x, position.y + 1.62, position.z) + direction * 0.5;

                        debug!(
                            "Arrow velocity: ({}, {}, {})",
                            velocity.x, velocity.y, velocity.z
                        );

                        debug!("Arrow Yaw: {}, Arrow Pitch: {}", **yaw, **pitch);

                        // Spawn arrow
                        world
                            .entity()
                            .add_enum(EntityKind::Arrow)
                            .set(Uuid::new_v4())
                            .set(Position::new(spawn_pos.x, spawn_pos.y, spawn_pos.z))
                            .set(Velocity::new(velocity.x, velocity.y, velocity.z))
                            .set(Pitch::new(**pitch))
                            .set(Yaw::new(**yaw))
                            .enqueue(Spawn);
                    },
                );
            }
        });

        system!(
            "arrow_entity_hit",
            world,
            &mut EventQueue<event::ProjectileEntityEvent>,
        )
        .singleton()
        .multi_threaded()
        .kind::<flecs::pipeline::PostUpdate>()
        .each_iter(move |it, _, event_queue| {
            let _system = it.system();
            let world = it.world();

            for event in event_queue.drain() {
                debug!("arrow_entity_hit: {event:?}");
                event.projectile.entity_view(world).get::<&Owner>(|owner| {
                    debug!("Sending attack event");
                    world.get::<&Events>(|events| {
                        events.push(
                            event::AttackEntity {
                                origin: owner.entity,
                                target: event.client,
                                damage: 1.0,
                            },
                            &world,
                        );
                    });

                    // Updating arrows in entity
                    debug!("Updating arrows in entity");
                    event
                        .client
                        .entity_view(world)
                        .get::<&mut ArrowsInEntity>(|arrows| {
                            arrows.0 += 1;
                        });
                });

                event.projectile.entity_view(world).destruct();
            }
        });
    }
}
