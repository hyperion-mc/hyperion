use std::net::SocketAddr;

use bevy::prelude::*;
use hyperion::{Crypto, Endpoint, HyperionCore, simulation::packet_state, spatial::Spatial};
use hyperion_proxy_module::SetProxyAddress;
use valence_text::IntoText;

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

impl Team {
    const fn name(self) -> &'static str {
        match self {
            Self::Black => "Black",
            Self::Blue => "Blue",
            Self::Brown => "Brown",
            Self::Cyan => "Cyan",
            Self::Gray => "Gray",
            Self::Green => "Green",
            Self::LightBlue => "Light Blue",
            Self::LightGray => "Light Gray",
            Self::Lime => "Lime",
            Self::Magenta => "Magenta",
            Self::Orange => "Orange",
            Self::Pink => "Pink",
            Self::Purple => "Purple",
            Self::Red => "Red",
            Self::White => "White",
            Self::Yellow => "Yellow",
        }
    }
}

impl From<Team> for valence_text::Color {
    fn from(team: Team) -> Self {
        // Source: https://minecraft.wiki/w/Wool/DV
        // (https://web.archive.org/web/20231011122724/https://minecraft.wiki/w/Wool/DV)
        match team {
            Team::Black => Self::rgb(0x14, 0x15, 0x19),
            Team::Blue => Self::rgb(0x35, 0x39, 0x9D),
            Team::Brown => Self::rgb(0x72, 0x47, 0x28),
            Team::Cyan => Self::rgb(0x15, 0x89, 0x91),
            Team::Gray => Self::rgb(0x3E, 0x44, 0x47),
            Team::Green => Self::rgb(0x54, 0x6D, 0x1B),
            Team::LightBlue => Self::rgb(0x3A, 0xAF, 0xD9),
            Team::LightGray => Self::rgb(0x8E, 0x8E, 0x86),
            Team::Lime => Self::rgb(0x70, 0xB9, 0x19),
            Team::Magenta => Self::rgb(0xBD, 0x44, 0xB3),
            Team::Orange => Self::rgb(0xF0, 0x76, 0x13),
            Team::Pink => Self::rgb(0xED, 0x8D, 0xAC),
            Team::Purple => Self::rgb(0x79, 0x2A, 0xAC),
            Team::Red => Self::rgb(0xA1, 0x27, 0x22),
            Team::White => Self::rgb(0xE9, 0xEC, 0xEC),
            Team::Yellow => Self::rgb(0xF8, 0xC6, 0x27),
        }
    }
}

impl From<Team> for valence_text::Text {
    fn from(team: Team) -> Self {
        team.name().into_text().color(team)
    }
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

pub fn init_game(address: SocketAddr, crypto: Crypto) -> anyhow::Result<()> {
    let mut app = App::new();

    app.insert_resource(Endpoint::from(address));
    app.insert_resource(crypto);
    app.add_plugins((HyperionCore, BedwarsPlugin));
    app.world_mut().trigger(SetProxyAddress {
        server: address.to_string(),
        ..SetProxyAddress::default()
    });

    app.run();

    Ok(())
}
