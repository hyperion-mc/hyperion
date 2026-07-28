use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldProvider};
use hyperion::{
    glam::Vec3,
    net::Channel,
    simulation::{
        Pitch, Position, Spawn, Uuid, Velocity, Yaw, entity_kind::EntityKind,
        projectile_motion::look_angles,
    },
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use tracing::debug;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "shoot")]
#[command_permission(group = "Normal")]
pub struct ShootCommand {
    #[arg(help = "Initial velocity of the arrow")]
    velocity: f32,
}

impl MinecraftCommand for ShootCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        const EYE_HEIGHT: f32 = 1.62;
        const BASE_VELOCITY: f32 = 3.0; // Base velocity multiplier for arrows

        let world = system.world();

        caller
            .entity_view(world)
            .get::<(&Position, &Yaw, &Pitch)>(|(pos, yaw, pitch)| {
                // Calculate direction vector from player's rotation
                let direction = super::raycast::get_direction_from_rotation(**yaw, **pitch);

                // Spawn arrow slightly in front of player to avoid self-collision
                let spawn_pos = Vec3::new(pos.x, pos.y + EYE_HEIGHT, pos.z) + direction * 1.0;

                // Calculate velocity with base multiplier
                let velocity = direction * (self.velocity * BASE_VELOCITY);

                // Facing is read off the velocity in vanilla's projectile
                // convention, not copied from the player's look; see
                // `look_angles`.
                let (arrow_yaw, arrow_pitch) = look_angles(velocity);

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
                world
                    .entity()
                    .add_enum(EntityKind::Arrow)
                    .set(entity_id)
                    .set(Position::new(spawn_pos.x, spawn_pos.y, spawn_pos.z))
                    .set(Velocity::new(velocity.x, velocity.y, velocity.z))
                    .set(Yaw::new(arrow_yaw))
                    .set(Pitch::new(arrow_pitch))
                    .add(Channel)
                    .enqueue(Spawn);
            });
    }
}
