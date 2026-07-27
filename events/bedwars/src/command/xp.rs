use clap::Parser;
use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldProvider, id};
use hyperion::simulation::Xp;
use hyperion_clap::{CommandPermission, MinecraftCommand};

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "xp")]
#[command_permission(group = "Admin")]
pub struct XpCommand {
    amount: u16,
}

impl MinecraftCommand for XpCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let Self { amount } = self;

        let world = system.world();
        let caller = caller.entity_view(world);

        caller.get::<&mut Xp>(|xp| {
            xp.amount = amount;
            caller.modified(id::<Xp>());
        });
    }
}
