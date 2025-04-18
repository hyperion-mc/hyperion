use flecs_ecs::{
    core::{QueryBuilderImpl, SystemAPI, World, WorldProvider, flecs, term::TermBuilderImpl},
    macros::Component,
    prelude::Module,
};
use hyperion::simulation::{Player, Xp};
use hyperion_inventory::PlayerInventory;
use hyperion_rank_tree::{Class, Team};
use tracing::debug;

use crate::MainBlockCount;

#[derive(Component)]
pub struct LevelModule;

#[derive(Component, Default, Copy, Clone, Debug)]
#[meta]
pub struct UpgradedTo {
    pub value: u8,
}

impl Module for LevelModule {
    #[allow(clippy::excessive_nesting)]
    fn module(world: &World) {
        world.component::<UpgradedTo>().meta();
        world
            .component::<Player>()
            .add_trait::<(flecs::With, UpgradedTo)>(); // todo: how does this even call Default? (IndraDb)

        world
            .observer::<flecs::OnSet, (
                &Xp,                  //                  (0)
                &UpgradedTo,          //                  (1)
                &Class,               //                  (2)
                &Team,                //                  (3)
                &MainBlockCount,      //                  (4)
                &mut PlayerInventory, //             (5)
            )>()
            .term_at(5u32)
            .filter()
            .each_entity(
                |entity, (xp, upgraded_to, rank, team, main_block_count, inventory)| {
                    debug!("updating level");
                    let new_level = xp.get_visual().level;
                    let world = entity.world();
                    let level_diff = new_level - upgraded_to.value;
                    rank.apply_inventory(*team, inventory, &world, **main_block_count, level_diff);
                },
            );
    }
}
