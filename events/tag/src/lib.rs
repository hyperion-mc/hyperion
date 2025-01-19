#![feature(allocator_api)]
#![feature(let_chains)]
#![feature(stmt_expr_attributes)]
#![feature(exact_size_is_empty)]

use std::{collections::HashSet, net::SocketAddr};

use flecs_ecs::prelude::*;
use hyperion::{GameServerEndpoint, HyperionCore, simulation::Player};
use hyperion_clap::hyperion_command::CommandRegistry;
use hyperion_gui::Gui;
use module::{block::BlockModule, vanish::VanishModule};

mod module;

use derive_more::{Deref, DerefMut};
use hyperion::{glam::IVec3, simulation::Position, spatial};
use hyperion_rank_tree::Team;
use module::{attack::AttackModule, level::LevelModule, regeneration::RegenerationModule};
use spatial::SpatialIndex;

use crate::{
    module::{bow::BowModule, chat::ChatModule, spawn::SpawnModule, stats::StatsModule},
    skin::SkinModule,
};

#[derive(Component)]
pub struct TagModule;

mod command;
mod skin;

#[derive(Component, Default, Deref, DerefMut)]
struct OreVeins {
    ores: HashSet<IVec3>,
}

#[derive(Component, Deref, DerefMut)]
struct MainBlockCount(i8);

impl Default for MainBlockCount {
    fn default() -> Self {
        Self(16)
    }
}

#[derive(Component)]
struct FollowClosestPlayer;

impl Module for TagModule {
    fn module(world: &World) {
        // on entity kind set UUID

        world.component::<FollowClosestPlayer>();
        world.component::<MainBlockCount>();
        world.component::<Gui>();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, MainBlockCount)>();

        world.import::<hyperion_rank_tree::RankTree>();

        world.component::<OreVeins>();
        world.set(OreVeins::default());

        world
            .component::<Player>()
            .add_trait::<(flecs::With, Team)>();

        world.import::<SpawnModule>();
        world.import::<ChatModule>();
        world.import::<StatsModule>();
        world.import::<BlockModule>();
        world.import::<hyperion_respawn::RespawnModule>();
        world.import::<AttackModule>();
        world.import::<LevelModule>();
        world.import::<BowModule>();
        world.import::<RegenerationModule>();
        world.import::<hyperion_permission::PermissionModule>();
        world.import::<hyperion_utils::HyperionUtilsModule>();
        world.import::<hyperion_clap::ClapCommandModule>();
        world.import::<SkinModule>();
        world.import::<VanishModule>();
        world.import::<hyperion_genmap::GenMapModule>();

        world.get::<&mut CommandRegistry>(|registry| {
            command::register(registry, world);
        });

        world.set(hyperion_utils::AppId {
            qualifier: "com".to_string(),
            organization: "andrewgazelka".to_string(),
            application: "hyperion-poc".to_string(),
        });

        // import spatial module and index all players
        world.import::<spatial::SpatialModule>();
        world
            .component::<Player>()
            .add_trait::<(flecs::With, spatial::Spatial)>();

        system!(
            "follow_closest_player",
            world,
            &SpatialIndex($),
            &mut Position,
        )
        .with::<FollowClosestPlayer>()
        .each_entity(|entity, (index, position)| {
            let world = entity.world();

            let Some(closest) = index.closest_to(**position, &world) else {
                return;
            };

            closest.get::<&Position>(|target_position| {
                let delta = **target_position - **position;

                if delta.length_squared() < 0.01 {
                    // we are already at the target position
                    return;
                }

                let delta = delta.normalize() * 0.1;

                **position += delta;
            });
        });
    }
}

pub fn init_game(address: SocketAddr) -> anyhow::Result<()> {
    let world = World::new();

    world.import::<HyperionCore>();
    world.import::<TagModule>();

    world.set(GameServerEndpoint::from(address));

    let mut app = world.app();

    app.enable_rest(0)
        .enable_stats(true)
        .set_threads(i32::try_from(rayon::current_num_threads())?);

    app.run();

    Ok(())
}
