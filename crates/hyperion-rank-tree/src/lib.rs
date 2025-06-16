use bevy::prelude::*;
use clap::ValueEnum;
use hyperion::simulation::packet_state;
use hyperion_inventory::PlayerInventory;
use hyperion_item::NbtInteractEvent;
use tracing::{debug, error};

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

pub struct RankTreePlugin;

#[derive(Resource)]
pub struct Handles {
    pub speed: Entity,
}

fn initialize_player(
    trigger: Trigger<'_, OnAdd, packet_state::Play>,
    mut commands: Commands<'_, '_>,
) {
    commands
        .entity(trigger.target())
        .insert(Team::default())
        .insert(Class::default());
}

fn handle_interact(
    mut events: EventReader<'_, '_, NbtInteractEvent>,
    handles: Res<'_, Handles>,
    query: Query<'_, '_, &PlayerInventory>,
) {
    for event in events.read() {
        if event.handler != handles.speed {
            continue;
        }

        let inventory = match query.get(event.client) {
            Ok(inventory) => inventory,
            Err(e) => {
                error!("failed to handle speed interact: query failed: {e}");
                continue;
            }
        };

        let cursor = inventory.get_cursor();
        debug!("clicked {cursor:?}");
    }
}

impl Plugin for RankTreePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_player);
        app.add_systems(FixedUpdate, handle_interact);

        let speed = app.world_mut().spawn_empty().id();
        app.insert_resource(Handles { speed });
    }
}
