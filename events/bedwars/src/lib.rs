use std::net::SocketAddr;

use flecs_ecs::prelude::*;
use hyperion::{
    Crypto, GameServerEndpoint, HyperionCore, hyperion_minecraft_proto::text::Rgb24,
    simulation::Player, spatial,
};
use hyperion_clap::hyperion_command::CommandRegistry;
use hyperion_gui::Gui;
use valence_text::IntoText;

use crate::module::{
    attack::AttackModule, block::BlockModule, bow::BowModule, chat::ChatModule,
    damage::DamageModule, regeneration::RegenerationModule, spawn::SpawnModule,
    tab_list::BedwarsTabListModule, vanish::VanishModule,
};

mod command;
mod module;

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
    /// The wool colour this team is drawn in.
    ///
    /// [`Rgb24`] rather than a packed `u32`, because the packed form has a
    /// quarter of its values outside anything `#RRGGBB` can spell and every
    /// consumer would have to decide what to do about that. Three channels are
    /// a colour by construction, so nothing here can fail.
    #[must_use]
    pub const fn rgb(self) -> Rgb24 {
        // Source: <https://minecraft.wiki/w/Wool/DV>
        // (<https://web.archive.org/web/20231011122724/https://minecraft.wiki/w/Wool/DV>)
        match self {
            Self::Black => Rgb24::new(0x14, 0x15, 0x19),
            Self::Blue => Rgb24::new(0x35, 0x39, 0x9D),
            Self::Brown => Rgb24::new(0x72, 0x47, 0x28),
            Self::Cyan => Rgb24::new(0x15, 0x89, 0x91),
            Self::Gray => Rgb24::new(0x3E, 0x44, 0x47),
            Self::Green => Rgb24::new(0x54, 0x6D, 0x1B),
            Self::LightBlue => Rgb24::new(0x3A, 0xAF, 0xD9),
            Self::LightGray => Rgb24::new(0x8E, 0x8E, 0x86),
            Self::Lime => Rgb24::new(0x70, 0xB9, 0x19),
            Self::Magenta => Rgb24::new(0xBD, 0x44, 0xB3),
            Self::Orange => Rgb24::new(0xF0, 0x76, 0x13),
            Self::Pink => Rgb24::new(0xED, 0x8D, 0xAC),
            Self::Purple => Rgb24::new(0x79, 0x2A, 0xAC),
            Self::Red => Rgb24::new(0xA1, 0x27, 0x22),
            Self::White => Rgb24::new(0xE9, 0xEC, 0xEC),
            Self::Yellow => Rgb24::new(0xF8, 0xC6, 0x27),
        }
    }
}

impl From<Team> for valence_text::Color {
    fn from(team: Team) -> Self {
        let [red, green, blue] = team.rgb().channels();
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
        world.import::<BedwarsTabListModule>();
        world.import::<BlockModule>();
        world.import::<AttackModule>();
        world.import::<BowModule>();
        world.import::<RegenerationModule>();
        world.import::<DamageModule>();
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
