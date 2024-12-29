use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldProvider};
use hyperion::{
    glam::Vec3,
    spatial::get_first_collision,
    simulation::{Pitch, Position, Yaw, entity_kind::EntityKind},
};
use hyperion_clap::{CommandPermission, MinecraftCommand};
use rayon::iter::Either;
use tracing::debug;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "raycast")]
#[command_permission(group = "Admin")]
pub struct RaycastCommand;

/// Converts Minecraft yaw and pitch angles to a direction vector
///
/// # Arguments
/// * `yaw` - The yaw angle in degrees (-180 to +180)
///   - -180° or +180°: facing North (negative Z)
///   - -90°: facing East (positive X)
///   - 0°: facing South (positive Z)
///   - +90°: facing West (negative X)
/// * `pitch` - The pitch angle in degrees (-90 to +90)
///   - -90°: looking straight up (positive Y)
///   - 0°: looking horizontal
///   - +90°: looking straight down (negative Y)
///
/// # Returns
/// A normalized Vec3 representing the look direction
pub fn get_direction_from_rotation(yaw: f32, pitch: f32) -> Vec3 {
    // Convert angles from degrees to radians
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();

    Vec3::new(
        -pitch_rad.cos() * yaw_rad.sin(), // x = -cos(pitch) * sin(yaw)
        -pitch_rad.sin(),                 // y = -sin(pitch)
        pitch_rad.cos() * yaw_rad.cos(),  // z = cos(pitch) * cos(yaw)
    )
}

impl MinecraftCommand for RaycastCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        const EYE_HEIGHT: f32 = 1.62;
        const DISTANCE: f32 = 10.0;

        let world = system.world();

        let ray =
            caller
                .entity_view(world)
                .get::<(&Position, &Yaw, &Pitch)>(|(position, yaw, pitch)| {
                    let center = **position;

                    let eye = center + Vec3::new(0.0, EYE_HEIGHT, 0.0);
                    let direction = get_direction_from_rotation(**yaw, **pitch);

                    geometry::ray::Ray::new(eye, direction) * DISTANCE
                });

        debug!("ray = {ray:?}");

        let result = get_first_collision(ray, &world);

        match result {
            Some(Either::Left(entity)) => {
                entity
                    .entity_view(world)
                    .get::<(&Position, &EntityKind)>(|(position, kind)| {
                        let position = **position;
                        debug!("kind: {kind:?}");
                        debug!("position: {position:?}");
                    });
            }
            Some(Either::Right(ray_collision)) => debug!("ray_collision: {ray_collision:?}"),
            None => debug!("no collision found"),
        }
    }
}
