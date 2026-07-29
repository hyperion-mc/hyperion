use std::time::{Duration, SystemTime};

use flecs_ecs::{
    core::{EntityViewGet, World},
    prelude::*,
};
use hyperion::{
    ItemKind, ItemStack,
    glam::Vec3,
    net::Channel,
    simulation::{
        Owner, Pitch, Player, Position, Spawn, Uuid, Velocity, Yaw,
        entity_kind::EntityKind,
        event::{self, ClientStatusCommand, ClientStatusEvent},
        get_direction_from_rotation,
        handlers::PacketSwitchQuery,
        metadata::living_entity::{ArrowsInEntity, HandStates},
        packet::HandlerRegistry,
        projectile_motion::{MAX_ARROW_SPEED, bow_power, look_angles, muzzle},
    },
    storage::{EventQueue, Events},
};
use hyperion_inventory::PlayerInventory;
use tracing::debug;

// The three numbers below and the curve in `BowCharging::get_charge` are all
// transcribed from one place:
//
//   net.minecraft.world.item.BowItem#getPowerForTime(int)
//   net.minecraft.world.item.BowItem#releaseUsing(ItemStack, Level, LivingEntity, int)
//
// Written down here rather than generated, because unlike a registry or a
// packet layout this is a method *body* and nothing in the toolchain reads
// those yet. That makes the citation the only thing standing between this and
// a silent drift on the next version bump, so it names the class and the exact
// signature: the point is that a reader can open the decompiled source and
// check the transcription rather than trust it.

/// Ticks in a second, for turning an elapsed `Duration` into vanilla's unit.
const TICKS_PER_SECOND: f32 = 20.0;

/// Ticks of holding that count as a full draw.
///
/// `getPowerForTime` divides the charge by 20 and caps the result at 1.0, so a
/// bow reaches full power one second after it is nocked. The same number as
/// [`TICKS_PER_SECOND`] and a different quantity: one is how fast the clock
/// runs, the other is how long the bow takes, and vanilla is free to change
/// either without the other.
const FULL_DRAW_TICKS: f32 = 20.0;

#[derive(Component)]
pub struct BowModule;

#[derive(Component)]
pub struct LastFireTime {
    pub time: SystemTime,
}

// (flecs::With, LastFireTime) below auto-adds this to every Player, so flecs
// needs to be able to construct one without our help.
impl Default for LastFireTime {
    fn default() -> Self {
        Self {
            time: SystemTime::UNIX_EPOCH,
        }
    }
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

#[derive(Component)]
pub struct BowCharging {
    pub start_time: SystemTime,
}

// Same reason as LastFireTime: it is a (flecs::With, _) target. Unlike
// LastFireTime the epoch is the wrong default here -- `get_charge` saturates a
// second after the bow is nocked, so a player who never drew it would release a
// full-power shot. "Started charging now" is the minimum charge.
impl Default for BowCharging {
    fn default() -> Self {
        Self::now()
    }
}

impl BowCharging {
    #[must_use]
    pub fn now() -> Self {
        Self {
            start_time: SystemTime::now(),
        }
    }

    /// How far the bow is drawn, as a fraction of a full draw.
    ///
    /// This is `getPowerForTime` and nothing else: the draw is counted in
    /// ticks, the curve is quadratic, and it saturates at 20 of them. So a full
    /// draw is one second, not the 1.2 this clamped to before, and because the
    /// result is a fraction the caller's `* 3.0` lands on exactly vanilla's
    /// maximum arrow speed.
    ///
    /// What it used to return was *seconds*, clamped to 1.2, which that same
    /// multiply turned into 3.6 blocks a tick -- a fifth faster than a real bow
    /// can shoot -- under a comment claiming 3.0 was the maximum.
    #[must_use]
    pub fn get_charge(&self) -> f32 {
        let elapsed = self.start_time.elapsed().unwrap_or(Duration::ZERO);
        let f = elapsed.as_secs_f32() * TICKS_PER_SECOND / FULL_DRAW_TICKS;
        bow_power(f)
    }
}

impl Module for BowModule {
    fn module(world: &World) {
        world.component::<LastFireTime>();
        world.component::<BowCharging>();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, LastFireTime)>()
            .add_trait::<(flecs::With, BowCharging)>();

        system!(
            "handle_bow_use",
            world,
            &mut EventQueue<event::ItemInteract>,
        )
        .kind(id::<flecs::pipeline::PostUpdate>())
        .each_iter(move |it, _, event_queue| {
            let world = it.world();

            for event in event_queue.drain() {
                event
                    .entity
                    .entity_view(world)
                    .get::<&PlayerInventory>(|inventory| {
                        if inventory.held().stack.item != ItemKind::Bow {
                            return;
                        }

                        event.entity.entity_view(world).set(BowCharging::now());
                        event.entity.entity_view(world).set(HandStates::new(1));
                    });
            }
        });

        system!(
            "handle_bow_release",
            world,
            &mut EventQueue<event::ReleaseUseItem>,
        )
        .kind(id::<flecs::pipeline::PreUpdate>())
        .each_iter(move |it, _, event_queue| {
            let world = it.world();

            for event in event_queue.drain() {
                if event.item != ItemKind::Bow {
                    continue;
                }

                let player = world.entity_from_id(event.from);

                // Check the cooldown
                let can_fire = player.get::<&LastFireTime>(LastFireTime::can_fire);

                if !can_fire {
                    continue;
                }

                // Update the last fire time
                player.set(LastFireTime::now());

                #[allow(clippy::excessive_nesting)]
                player.get::<(&mut PlayerInventory, &Position, &Yaw, &Pitch)>(
                    |(inventory, position, yaw, pitch)| {
                        debug!("Player {} released the bow", player.id());
                        // Check if the player has enough arrows in their inventory
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
                        let charge = player.get::<&BowCharging>(BowCharging::get_charge);

                        debug!(
                            "Player {} fired an arrow with charge {}",
                            player.id(),
                            charge
                        );

                        // Calculate the direction vector from the player's rotation
                        let direction = get_direction_from_rotation(**yaw, **pitch);
                        let velocity = direction * (charge * MAX_ARROW_SPEED);

                        // The arrow's own facing is read off its velocity in
                        // vanilla's projectile convention, not copied from the
                        // shooter's look yaw: the two share a magnitude but not a
                        // sign, and sending the player's yaw is what renders the
                        // arrow pointing the wrong way. `update_projectile_positions`
                        // keeps it aimed from here; this is only the launch seed.
                        let (arrow_yaw, arrow_pitch) = look_angles(velocity);

                        let spawn_pos = muzzle(**position, direction);

                        debug!("Arrow spawn position: {:?}", spawn_pos);

                        world
                            .entity()
                            .add_enum(EntityKind::Arrow)
                            .set(Uuid::new_v4())
                            .set(Position::new(spawn_pos.x, spawn_pos.y, spawn_pos.z))
                            .set(Velocity::new(velocity.x, velocity.y, velocity.z))
                            .set(Pitch::new(arrow_pitch))
                            .set(Yaw::new(arrow_yaw))
                            .set(Owner::new(*player))
                            .add(Channel)
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
        .kind(id::<flecs::pipeline::PostUpdate>())
        .each_iter(move |it, _, event_queue| {
            let world = it.world();

            for event in event_queue.drain() {
                let (damage, owner) = event
                    .projectile
                    .entity_view(world)
                    .get::<(&Velocity, &Owner)>(|(velocity, owner)| {
                        (velocity.0.length() * 2.0, owner.entity)
                    });

                // Two separate refusals, and they were one `&&` before, which
                // meant an arrow was only ever ignored when it was both
                // stationary *and* the victim's own -- so a player's own moving
                // arrow damaged them, and any arrow at rest still counted as a
                // hit against everybody else.
                //
                // An arrow that has stopped has already been pinned into a
                // block by `arrow_block_hit`, and a stuck arrow is scenery.
                if damage == 0.0 {
                    continue;
                }

                // Vanilla gives an arrow a few ticks of immunity to its own
                // shooter, because it spawns 0.5 blocks in front of the eyes and
                // would otherwise clip the hitbox it came out of on tick one.
                // Nothing here integrates that grace period, so the owner is
                // simply never a valid target.
                if owner == event.client {
                    continue;
                }

                event
                    .client
                    .entity_view(world)
                    .get::<&mut ArrowsInEntity>(|arrows| {
                        arrows.0 += 1;
                    });

                event.projectile.entity_view(world).destruct();

                world.get::<&Events>(|events| {
                    events.push(
                        event::AttackEntity {
                            origin: owner,
                            target: event.client,
                            damage,
                        },
                        &world,
                    );
                });
            }
        });

        // multi-threaded causes issues
        system!(
            "arrow_block_hit",
            world,
            &mut EventQueue<event::ProjectileBlockEvent>,
        )
        .kind(id::<flecs::pipeline::PreStore>())
        .each_iter(move |it, _, event_queue| {
            let world = it.world();

            for event in event_queue.drain() {
                event
                    .projectile
                    .entity_view(world)
                    .get::<(&mut Position, &mut Velocity)>(|(position, velocity)| {
                        debug!("Arrow hit block at {:?}", event.collision.point);
                        velocity.0 = Vec3::ZERO;
                        **position = event.collision.point;
                    });
            }
        });

        // Arrows stuck in a player are cosmetic, and until now they were
        // permanent: `arrow_entity_hit` only ever incremented this, so a player
        // who took six arrows over a match wore all six for the rest of it and
        // through every respawn after. Vanilla bleeds them off over time and
        // clears them outright on respawn; this does the clearing, which is the
        // half that stops the count growing without bound.
        //
        // A `HandlerRegistry` handler and not a system on
        // `EventQueue<ClientStatusEvent>`, because `EventQueue::drain` is
        // destructive and single-consumer: a second drain of that queue would
        // race `attack.rs` for the same events and one of the two would
        // silently stop seeing respawns. The registry fans every handler out
        // over the same value instead.
        world.get::<&mut HandlerRegistry>(|registry| {
            registry.add_handler(Box::new(
                |status: &ClientStatusEvent, query: &mut PacketSwitchQuery<'_>| {
                    if status.status == ClientStatusCommand::RequestStats {
                        return Ok(());
                    }

                    status
                        .client
                        .entity_view(query.world)
                        .get::<&mut ArrowsInEntity>(|arrows| {
                            arrows.0 = 0;
                        });

                    Ok(())
                },
            ));
        });
    }
}
