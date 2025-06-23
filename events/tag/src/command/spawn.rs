use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::{
    // BlockState,
    simulation::{
        Pitch,
        Position,
        SpawnEvent,
        //        metadata::{
        //            block_display::DisplayedBlockState,
        //            display::{Height, Width},
        //        },
        Uuid,
        Velocity,
        Yaw,
        entity_kind::EntityKind,
    },
};
use hyperion_clap::{CommandPermission, MinecraftCommand};

use crate::FollowClosestPlayer;

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "spawn")]
#[command_permission(group = "Normal")]
pub struct SpawnCommand;

impl MinecraftCommand for SpawnCommand {
    type State = SystemState<Commands<'static, 'static>>;

    fn execute(self, world: &World, state: &mut Self::State, _caller: Entity) {
        let mut commands = state.get(world);

        // TODO: add missing components
        let entity = commands
            .spawn((
                EntityKind::BlockDisplay,
                // EntityFlags::ON_FIRE
                Uuid::new_v4(),
                // Width::new(1.0),
                // Height::new(1.0),
                // ViewRange::new(100.0)
                // EntityKind::Zombie
                Position::new(0.0, 22.0, 0.0),
                Pitch::new(0.0),
                Yaw::new(0.0),
                Velocity::new(0.0, 0.0, 0.0),
                FollowClosestPlayer,
                // DisplayedBlockState::new(BlockState::DIRT)
            ))
            .id();

        commands.queue(move |world: &mut World| {
            let mut events = world.resource_mut::<Events<SpawnEvent>>();
            events.send(SpawnEvent(entity));
        });
    }
}
