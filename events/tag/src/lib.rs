#![feature(allocator_api)]
#![feature(let_chains)]
#![feature(stmt_expr_attributes)]
#![feature(exact_size_is_empty)]

use std::{collections::HashSet, net::SocketAddr};

use bevy::prelude::*;
use hyperion::{
    HyperionCore, SetEndpoint,
    simulation::{EntitySize, Position, packet_state},
    spatial::{Spatial, SpatialIndex},
};
use hyperion_proxy_module::SetProxyAddress;
use tracing::error;

use crate::{
    plugin::{
        attack::AttackPlugin, block::BlockPlugin, bow::BowPlugin, chat::ChatPlugin,
        damage::DamagePlugin, level::LevelPlugin, regeneration::RegenerationPlugin,
        spawn::SpawnPlugin, stats::StatsPlugin, vanish::VanishPlugin,
    },
    skin::SkinPlugin,
};

mod command;
mod plugin;
mod skin;

#[derive(Resource, Default, Deref, DerefMut)]
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

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert((Spatial, MainBlockCount::default()));
}

fn follow_closest_player(
    index: Res<'_, SpatialIndex>,
    follow_query: Query<'_, '_, Entity, With<FollowClosestPlayer>>,
    mut queries: ParamSet<
        '_,
        '_,
        (
            Query<'_, '_, &mut Position>,
            Query<'_, '_, (&Position, &EntitySize)>,
        ),
    >,
) {
    for entity in follow_query.iter() {
        let position = match queries.p0().get(entity) {
            Ok(position) => **position,
            Err(e) => {
                error!("follow closest player failed: query failed: {e}");
                continue;
            }
        };

        let Some(closest) = index.closest_to(position, queries.p1()) else {
            continue;
        };

        let target_position = match queries.p0().get(closest) {
            Ok(position) => **position,
            Err(e) => {
                error!("follow closest player failed: query failed: {e}");
                continue;
            }
        };

        let delta = target_position - position;

        if delta.length_squared() < 0.01 {
            // we are already at the target position
            return;
        }

        let delta = delta.normalize() * 0.1;

        match queries.p0().get_mut(entity) {
            Ok(mut position) => {
                **position += delta;
            }
            Err(e) => {
                error!("follow closest player failed: query failed: {e}");
            }
        }
    }
}

#[derive(Component)]
pub struct TagPlugin;

impl Plugin for TagPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(OreVeins::default());
        app.add_plugins((
            (
                AttackPlugin,
                BlockPlugin,
                BowPlugin,
                ChatPlugin,
                DamagePlugin,
                LevelPlugin,
                RegenerationPlugin,
                SkinPlugin,
                SpawnPlugin,
                StatsPlugin,
                VanishPlugin,
            ),
            hyperion_clap::ClapCommandPlugin,
            hyperion_genmap::GenMapPlugin,
            hyperion_item::ItemPlugin,
            hyperion_permission::PermissionPlugin,
            hyperion_rank_tree::RankTreePlugin,
            hyperion_respawn::RespawnPlugin,
            hyperion_proxy_module::HyperionProxyPlugin,
        ));
        app.add_observer(initialize_player);
        app.add_systems(FixedUpdate, follow_closest_player);

        command::register(app.world_mut());
    }
}

pub fn init_game(address: SocketAddr) -> anyhow::Result<()> {
    let mut app = App::new();

    app.add_plugins((HyperionCore, TagPlugin));
    app.world_mut().trigger(SetEndpoint::from(address));
    app.world_mut().trigger(SetProxyAddress {
        server: address.to_string(),
        ..SetProxyAddress::default()
    });

    app.run();

    Ok(())
}
