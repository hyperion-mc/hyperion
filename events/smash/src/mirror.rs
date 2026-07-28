//! The read half of the seam: hyperion's state copied onto the game's mirrors.
//!
//! Nothing here goes the other way. The game's per-tick hot paths -- the arena
//! bounds check, cooldowns, projectile integration -- are plain component
//! iteration precisely because position, facing and ground state arrive as
//! components rather than as trait calls, and that only holds if one system
//! owns the copy.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::{MovementTracking, Pitch, Yaw};
use hyperion_inventory::PlayerInventory;

use crate::{
    input::hotbar_slot,
    module::player::{Facing, OnGround, Player, Position, SelectedSlot, Velocity},
};

#[derive(Component)]
pub struct MirrorModule;

impl Module for MirrorModule {
    fn module(world: &World) {
        // OnLoad, ahead of every game system: a tick that read last tick's
        // position would resolve knockback against a stale origin.
        world
            .system_named::<(
                &hyperion::simulation::Position,
                &Yaw,
                &Pitch,
                &MovementTracking,
                &PlayerInventory,
                &mut Position,
                &mut Facing,
                &mut OnGround,
                &mut Velocity,
                &mut SelectedSlot,
            )>("smash::mirror_player_state")
            .kind(id::<flecs::pipeline::OnLoad>())
            .with(Player::id())
            .each(
                |(
                    source,
                    yaw,
                    pitch,
                    tracking,
                    inventory,
                    position,
                    facing,
                    ground,
                    velocity,
                    selected,
                )| {
                    position.0 = **source;
                    facing.0 = look_vector(**yaw, **pitch);
                    ground.0 = tracking.was_on_ground;
                    // hyperion zeroes `Velocity` every tick once it has been sent
                    // to the client, so the component itself is almost always
                    // zero when read. `server_velocity` is the running estimate,
                    // and it is what abilities that scale with speed -- Block
                    // Toss, Iron Hook -- are asking for.
                    velocity.0 = tracking.server_velocity.as_vec3();
                    // A cursor outside the hotbar leaves the last slot standing
                    // rather than resetting to zero: the alternative reads as
                    // "you are holding slot 0" and would put slot 0's cooldown
                    // on the experience bar of somebody holding nothing.
                    if let Some(slot) = hotbar_slot(inventory.held_slot()) {
                        selected.0 = slot;
                    }
                },
            );
    }
}

/// Unit vector a player with this yaw and pitch is looking along.
///
/// Minecraft's yaw is degrees clockwise from south (+Z) and pitch is degrees
/// downward from the horizon, which is neither of the two conventions glam has
/// a helper for.
#[must_use]
pub fn look_vector(yaw: f32, pitch: f32) -> Vec3 {
    let yaw = yaw.to_radians();
    let pitch = pitch.to_radians();
    Vec3::new(
        -pitch.cos() * yaw.sin(),
        -pitch.sin(),
        pitch.cos() * yaw.cos(),
    )
}
