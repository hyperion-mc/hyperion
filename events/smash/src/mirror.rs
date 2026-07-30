//! The read half of the seam: hyperion's state copied onto the game's mirrors.
//!
//! Nothing here goes the other way. The game's per-tick hot paths -- the arena
//! bounds check, cooldowns, projectile integration -- are plain component
//! iteration precisely because position, facing and ground state arrive as
//! components rather than as trait calls, and that only holds if one system
//! owns the copy.
//!
//! # A mirror cannot compute a rising edge
//!
//! Read this before adding a mirror of a host *bit*, because the obvious thing
//! to write does not work and does not fail loudly.
//!
//! This system runs in `OnLoad`. Two things that decide what a bit looks like
//! happen strictly after it, every tick:
//!
//! | phase | what happens | where |
//! |---|---|---|
//! | `OnLoad` | **this system copies** | here |
//! | `OnUpdate` | hyperion decodes the tick's packets | `ingress/decode.rs:81` |
//! | `OnUpdate` | the game's own systems run | `smash::*` |
//! | `PostUpdate` | the adapter drains the write queue | `adapter.rs` |
//! | `PreStore` | `is_flying` is copied to `last_tick_flying` | `sync_entity_state.rs:471` |
//!
//! So `bit && !hyperions_previous_bit` is always false here. The packet that
//! sets the bit arrives after this system has already run, and by the time it
//! runs again hyperion's own "previous" has caught up with it. There is no
//! moment in the tick at which this system can see the two disagree.
//!
//! That is exactly the bug [`crate::module::player::Flying`] was written wrong
//! for the first time, and the way it surfaced is the part worth remembering:
//! **every Rust test passed.** They drive the mirrored component directly, so
//! they never exercise the mirror at all, and a mirror that can only ever
//! write `false` is invisible to all of them. What found it was
//! `Match.prove_double_jump` in `tools/smash-match.py` -- a real client
//! double-tapping jump and getting no impulse back.
//!
//! Two ways out, and the second is the one taken here. Keep the edge somewhere
//! that is written after the packet is decoded, or **mirror the level and make
//! the consumer idempotent**: [`crate::module::jump`] answers a press by
//! clearing the host bit in the same tick and refuses one from a player
//! standing on the ground, so reading a level as an edge is sound. If you
//! mirror a new bit, say which of the two you did and why.

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::simulation::{Flight, MovementTracking, Pitch, Yaw};
use hyperion_inventory::PlayerInventory;

use crate::{
    input::hotbar_slot,
    module::player::{Facing, Flying, OnGround, Player, Position, SelectedSlot, Velocity},
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
                &Flight,
                &PlayerInventory,
                &mut Position,
                &mut Facing,
                &mut OnGround,
                &mut Velocity,
                &mut Flying,
                &mut SelectedSlot,
            )>("mirror_player_state")
            .kind(id::<flecs::pipeline::OnLoad>())
            .with(Player::id())
            .each(
                |(
                    source,
                    yaw,
                    pitch,
                    tracking,
                    flight,
                    inventory,
                    position,
                    facing,
                    ground,
                    velocity,
                    flying,
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
                    // A plain copy, deliberately. An edge computed here is
                    // always false -- see this module's own documentation for
                    // the phase ordering that makes it so, and `Flying` for
                    // why a level is safe to read as a press instead. One tick
                    // of latency, the same one knockback already costs.
                    flying.0 = flight.is_flying;
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
