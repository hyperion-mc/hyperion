use clap::ValueEnum;
use flecs_ecs::{
    core::{Entity, IdOperations, World, flecs},
    macros::Component,
    prelude::Module,
};
use hyperion::{
    simulation::{Player, handlers::PacketSwitchQuery},
    storage::{EventFn, InteractEvent},
};

pub mod inventory;

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq, Component, Default)]
#[repr(C)]
pub enum Class {
    /// ![Widget Example](https://i.imgur.com/pW7v0Xn.png)
    ///
    /// The stick is the starting rank.
    #[default]
    Stick, // -> [Pickaxe | Sword | Bow ]

    Archer,
    Sword,
    Miner,

    Excavator,

    Mage,
    Knight,
    Builder,
}

#[derive(
    Copy, Clone, Debug, PartialEq, Eq, ValueEnum, PartialOrd, Ord, Component, Default
)]
pub enum Team {
    #[default]
    Blue,
    Green,
    Red,
    Yellow,
}

#[derive(Component)]
pub struct RankTree;

#[derive(Component)]
pub struct Handles {
    pub speed: Entity,
}

impl Module for RankTree {
    fn module(world: &World) {
        world.import::<hyperion_item::ItemModule>();
        world.component::<Team>();
        world.component::<Class>();
        world.component::<Handles>();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, Team)>();

        world
            .component::<Player>()
            .add_trait::<(flecs::With, Class)>();

        let handler: EventFn<InteractEvent> = Box::new(|query: &mut PacketSwitchQuery<'_>, _| {
            let cursor = query.inventory.get_cursor();
            tracing::debug!("clicked {cursor:?}");
        });

        let speed = world.entity().set(hyperion_item::Handler::new(handler));

        world.set(Handles { speed: speed.id() });
    }
}
