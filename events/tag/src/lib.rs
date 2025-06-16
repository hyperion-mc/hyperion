#![feature(allocator_api)]
#![feature(let_chains)]
#![feature(stmt_expr_attributes)]
#![feature(exact_size_is_empty)]

use std::net::SocketAddr;

use bevy::prelude::*;
use hyperion::{
    HyperionCore,
    SetEndpoint,
    // simulation::{Player, Position},
    // spatial,
};

// use hyperion_clap::hyperion_command::CommandRegistry;
// use hyperion_gui::Gui;
// use hyperion_proxy_module::{HyperionProxyModule, ProxyAddress};
// use hyperion_rank_tree::Team;
// use module::{
//     attack::AttackModule, block::BlockModule, damage::DamageModule, level::LevelModule,
//     regeneration::RegenerationModule, vanish::VanishModule,
// };
// use spatial::SpatialIndex;
use crate::module::{spawn::SpawnPlugin, stats::StatsPlugin};
// use crate::{
//     module::{bow::BowModule, chat::ChatModule, spawn::SpawnModule, stats::StatsModule},
//     skin::SkinModule,
// };

mod module;
// mod command;
// mod skin;

// #[derive(Component, Default, Deref, DerefMut)]
// struct OreVeins {
//     ores: HashSet<IVec3>,
// }
//
// #[derive(Component, Deref, DerefMut)]
// struct MainBlockCount(i8);
//
// impl Default for MainBlockCount {
//     fn default() -> Self {
//         Self(16)
//     }
// }
//
// #[derive(Component)]
// struct FollowClosestPlayer;
//
// impl Module for TagModule {
//     fn module(world: &World) {
//         // on entity kind set UUID
//
//         world.component::<FollowClosestPlayer>();
//         world.component::<MainBlockCount>();
//         world.component::<Gui>();
//
//         world
//             .component::<Player>()
//             .add_trait::<(flecs::With, MainBlockCount)>();
//
//         world.import::<hyperion_rank_tree::RankTree>();
//
//         world.component::<OreVeins>();
//         world.set(OreVeins::default());
//
//         world
//             .component::<Player>()
//             .add_trait::<(flecs::With, Team)>();
//
//         world.import::<SpawnModule>();
//         world.import::<ChatModule>();
//         world.import::<StatsModule>();
//         world.import::<BlockModule>();
//         world.import::<hyperion_respawn::RespawnModule>();
//         world.import::<AttackModule>();
//         world.import::<LevelModule>();
//         world.import::<BowModule>();
//         world.import::<RegenerationModule>();
//         world.import::<hyperion_permission::PermissionModule>();
//         world.import::<hyperion_utils::HyperionUtilsModule>();
//         world.import::<hyperion_clap::ClapCommandModule>();
//         world.import::<SkinModule>();
//         world.import::<VanishModule>();
//         world.import::<DamageModule>();
//
//         world.get::<&mut CommandRegistry>(|registry| {
//             command::register(registry, world);
//         });
//
//         world.set(hyperion_utils::AppId {
//             qualifier: "com".to_string(),
//             organization: "andrewgazelka".to_string(),
//             application: "hyperion-poc".to_string(),
//         });
//
//         // import spatial module and index all players
//         world.import::<spatial::SpatialModule>();
//         world
//             .component::<Player>()
//             .add_trait::<(flecs::With, spatial::Spatial)>();
//
//         system!(
//             "follow_closest_player",
//             world,
//             &SpatialIndex($),
//             &mut Position,
//         )
//         .with(id::<FollowClosestPlayer>())
//         .each_entity(|entity, (index, position)| {
//             let world = entity.world();
//
//             let Some(closest) = index.closest_to(**position, &world) else {
//                 return;
//             };
//
//             closest.get::<&Position>(|target_position| {
//                 let delta = **target_position - **position;
//
//                 if delta.length_squared() < 0.01 {
//                     // we are already at the target position
//                     return;
//                 }
//
//                 let delta = delta.normalize() * 0.1;
//
//                 **position += delta;
//             });
//         });
//     }
// }

#[derive(Component)]
pub struct TagPlugin;

impl Plugin for TagPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            StatsPlugin,
            SpawnPlugin,
            hyperion_genmap::GenMapPlugin,
            hyperion_item::ItemPlugin,
            hyperion_rank_tree::RankTreePlugin,
        ));
    }
}

pub fn init_game(address: SocketAddr) -> anyhow::Result<()> {
    let mut app = App::new();

    app.add_plugins((HyperionCore, TagPlugin));
    app.world_mut().trigger(SetEndpoint::from(address));

    app.run();

    Ok(())
}
