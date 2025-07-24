#![feature(allocator_api)]
#![feature(let_chains)]
#![feature(stmt_expr_attributes)]
#![feature(exact_size_is_empty)]

use std::net::SocketAddr;

use bevy::prelude::*;
use hyperion::{HyperionCore, SetEndpoint, simulation::packet_state, spatial::Spatial};
use hyperion_proxy_module::SetProxyAddress;

use crate::{
    plugin::{
        attack::AttackPlugin, block::BlockPlugin, bow::BowPlugin, chat::ChatPlugin,
        damage::DamagePlugin, regeneration::RegenerationPlugin, spawn::SpawnPlugin,
        stats::StatsPlugin, vanish::VanishPlugin,
    },
    skin::SkinPlugin,
};

mod command;
mod plugin;
mod skin;

#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub enum Team {
    // Sorted alphabetically
    Black,
    Blue,
    Brown,
    Cyan,
    Gray,
    Green,
    LightBlue,
    LightGray,
    Lime,
    Magenta,
    Orange,
    Pink,
    Purple,
    Red,
    White,
    Yellow,
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert((Spatial, Team::Red));
}

#[derive(Component)]
pub struct BedwarsPlugin;

impl Plugin for BedwarsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            (
                AttackPlugin,
                BlockPlugin,
                BowPlugin,
                ChatPlugin,
                DamagePlugin,
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
            hyperion_proxy_module::HyperionProxyPlugin,
        ));
        app.add_observer(initialize_player);

        command::register(app.world_mut());
    }
}

pub fn init_game(address: SocketAddr) -> anyhow::Result<()> {
    let mut app = App::new();

    app.add_plugins((HyperionCore, BedwarsPlugin));
    app.world_mut().trigger(SetEndpoint::from(address));
    app.world_mut().trigger(SetProxyAddress {
        server: address.to_string(),
        ..SetProxyAddress::default()
    });

    app.run();

    Ok(())
}
