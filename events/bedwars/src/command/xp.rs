use bevy::{ecs::system::SystemState, prelude::*};
use clap::Parser;
use hyperion::simulation::Xp;
use hyperion_clap::{CommandPermission, MinecraftCommand};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "xp")]
#[command_permission(group = "Admin")]
pub struct XpCommand {
    amount: u16,
}

impl MinecraftCommand for XpCommand {
    type State = SystemState<Commands<'static, 'static>>;

    fn execute(self, world: &World, state: &mut Self::State, caller: Entity) {
        let mut commands = state.get(world);
        commands.entity(caller).insert(Xp {
            amount: self.amount,
        });
    }
}
