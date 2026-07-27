#![feature(exact_size_is_empty)]

use std::net::SocketAddr;

use flecs_ecs::prelude::*;
use hyperion::{Crypto, GameServerEndpoint, HyperionCore, simulation::Player, spatial};
use hyperion_clap::hyperion_command::CommandRegistry;
use hyperion_gui::Gui;
use valence_text::IntoText;

use crate::{
    module::{
        attack::AttackModule, block::BlockModule, bow::BowModule, chat::ChatModule,
        damage::DamageModule, regeneration::RegenerationModule, spawn::SpawnModule,
        tab_list::TabListModule, vanish::VanishModule,
    },
    skin::SkinModule,
};

mod command;
mod module;
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

impl Team {
    /// The wool colour this team is drawn in, as packed `0xRRGGBB`.
    #[must_use]
    pub const fn rgb(self) -> u32 {
        // Source: <https://minecraft.wiki/w/Wool/DV>
        // (<https://web.archive.org/web/20231011122724/https://minecraft.wiki/w/Wool/DV>)
        match self {
            Self::Black => 0x0014_1519,
            Self::Blue => 0x0035_399D,
            Self::Brown => 0x0072_4728,
            Self::Cyan => 0x0015_8991,
            Self::Gray => 0x003E_4447,
            Self::Green => 0x0054_6D1B,
            Self::LightBlue => 0x003A_AFD9,
            Self::LightGray => 0x008E_8E86,
            Self::Lime => 0x0070_B919,
            Self::Magenta => 0x00BD_44B3,
            Self::Orange => 0x00F0_7613,
            Self::Pink => 0x00ED_8DAC,
            Self::Purple => 0x0079_2AAC,
            Self::Red => 0x00A1_2722,
            Self::White => 0x00E9_ECEC,
            Self::Yellow => 0x00F8_C627,
        }
    }
}

impl From<Team> for valence_text::Color {
    fn from(team: Team) -> Self {
        let [_, red, green, blue] = team.rgb().to_be_bytes();
        Self::rgb(red, green, blue)
    }
}

impl From<Team> for valence_text::Text {
    fn from(team: Team) -> Self {
        team.name().into_text().color(team)
    }
}

#[derive(Component)]
pub struct BedwarsModule;

impl Module for BedwarsModule {
    fn module(world: &World) {
        world.component::<Team>();
        world.component::<Gui>();

        world.import::<SpawnModule>();
        world.import::<ChatModule>();
        world.import::<TabListModule>();
        world.import::<BlockModule>();
        world.import::<AttackModule>();
        world.import::<BowModule>();
        world.import::<RegenerationModule>();
        world.import::<DamageModule>();
        world.import::<SkinModule>();
        world.import::<VanishModule>();
        world.import::<hyperion_permission::PermissionModule>();
        world.import::<hyperion_utils::HyperionUtilsModule>();
        world.import::<hyperion_clap::ClapCommandModule>();
        world.import::<hyperion_genmap::GenMapModule>();
        world.import::<hyperion_item::ItemModule>();

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

        // Every player is spatially indexed and starts on the red team.
        world
            .observer::<flecs::OnAdd, ()>()
            .with(id::<Player>())
            .each_entity(|entity, ()| {
                entity.add(id::<spatial::Spatial>());
                entity.set(Team::Red);
            });
    }
}

pub fn init_game(address: SocketAddr, crypto: Crypto) -> anyhow::Result<()> {
    let world = World::new();

    world.import::<HyperionCore>();
    world.import::<BedwarsModule>();

    world.set(crypto);
    world.set(GameServerEndpoint::from(address));

    let mut app = world.app();

    app.enable_rest(0)
        .enable_stats(true)
        .set_threads(i32::try_from(rayon::current_num_threads())?);

    app.run();

    Ok(())
}
