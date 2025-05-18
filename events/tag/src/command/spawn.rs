use clap::Parser;
use flecs_ecs::{
    core::{Entity, WorldProvider, id},
    prelude::EntityView,
};
use hyperion::{
    BlockState,
    simulation::{
        Pitch, Position, Spawn, Uuid, Velocity, Yaw,
        entity_kind::EntityKind,
        metadata::{
            block_display::DisplayedBlockState,
            display::{Height, Width},
        },
    },
};
use hyperion_clap::{CommandPermission, MinecraftCommand};

use crate::FollowClosestPlayer;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "spawn")]
#[command_permission(group = "Normal")]
pub struct SpawnCommand;

impl MinecraftCommand for SpawnCommand {
    fn execute(self, system: EntityView<'_>, _caller: Entity) {
        let world = system.world();

        world
            .entity()
            .add_enum(EntityKind::BlockDisplay)
            // .set(EntityFlags::ON_FIRE)
            .set(Uuid::new_v4())
            .set(Width::new(1.0))
            .set(Height::new(1.0))
            // .set(ViewRange::new(100.0))
            // .add_enum(EntityKind::Zombie)
            .set(Position::new(0.0, 22.0, 0.0))
            .set(Pitch::new(0.0))
            .set(Yaw::new(0.0))
            .set(Velocity::new(0.0, 0.0, 0.0))
            .add(id::<FollowClosestPlayer>())
            .set(DisplayedBlockState::new(BlockState::DIRT))
            // .is_a(prefabs.block_display_base)
            .enqueue(Spawn);
    }
}
