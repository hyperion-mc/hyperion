use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{
    glam::Vec3,
    simulation::{Pitch, Position, SpawnEvent, Uuid, Velocity, Yaw, entity_kind::EntityKind},
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use tracing::{debug, error};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "shoot")]
#[command_permission(group = "Normal")]
pub struct ShootCommand {
    #[arg(help = "Initial velocity of the arrow")]
    velocity: f32,
}

impl MinecraftCommand for ShootCommand {
    type State = SystemState<(
        Query<'static, 'static, (&'static Position, &'static Yaw, &'static Pitch)>,
        Commands<'static, 'static>,
    )>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        const EYE_HEIGHT: f32 = 1.62;
        const BASE_VELOCITY: f32 = 3.0; // Base velocity multiplier for arrows

        let (query, mut commands) = state.get(world);

        let (pos, yaw, pitch) = match query.get(caller) {
            Ok(data) => data,
            Err(e) => {
                error!("shoot command failed: query failed: {e}");
                return;
            }
        };

        // Calculate direction vector from player's rotation
        let direction = super::raycast::get_direction_from_rotation(**yaw, **pitch);

        // Spawn arrow slightly in front of player to avoid self-collision
        let spawn_pos = Vec3::new(pos.x, pos.y + EYE_HEIGHT, pos.z) + direction * 1.0;

        // Calculate velocity with base multiplier
        let velocity = direction * (self.velocity * BASE_VELOCITY);

        debug!(
            "Arrow velocity: ({}, {}, {})",
            velocity.x, velocity.y, velocity.z
        );

        debug!(
            "Arrow spawn position: ({}, {}, {})",
            spawn_pos.x, spawn_pos.y, spawn_pos.z
        );

        let entity_id = Uuid::new_v4();

        // Create arrow entity with velocity
        let entity = commands
            .spawn((
                EntityKind::Arrow,
                entity_id,
                Position::new(spawn_pos.x, spawn_pos.y, spawn_pos.z),
                Velocity::new(velocity.x, velocity.y, velocity.z),
                Yaw::new(**yaw),
                Pitch::new(**pitch),
            ))
            .id();

        commands.queue(move |world: &mut World| {
            let mut events = world.resource_mut::<Events<SpawnEvent>>();
            events.send(SpawnEvent(entity));
        });
    }
}
