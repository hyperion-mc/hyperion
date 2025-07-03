mod storage;

use bevy::{ecs::world::OnDespawn, prelude::*};
use clap::ValueEnum;
use hyperion::{
    net::{Compose, ConnectionId},
    simulation::{Uuid, command::get_command_packet},
    storage::LocalDb,
};
use num_derive::{FromPrimitive, ToPrimitive};
use storage::PermissionStorage;
use tracing::error;

pub struct PermissionPlugin;

#[derive(
    Default,
    Component,
    FromPrimitive,
    ToPrimitive,
    Copy,
    Clone,
    Debug,
    PartialEq,
    ValueEnum,
    Eq
)]
#[repr(C)]
pub enum Group {
    Banned,
    #[default]
    Normal,
    Moderator,
    Admin,
}

// todo:

fn load_permissions(
    trigger: Trigger<'_, OnAdd, Uuid>,
    query: Query<'_, '_, &Uuid, With<ConnectionId>>,
    permissions: Res<'_, PermissionStorage>,
    mut commands: Commands<'_, '_>,
) {
    let Ok(uuid) = query.get(trigger.target()) else {
        return;
    };

    let group = permissions.get(**uuid);
    commands.entity(trigger.target()).insert(group);
}

fn store_permissions(
    trigger: Trigger<'_, OnDespawn, Group>,
    query: Query<'_, '_, (&Uuid, &Group)>,
    permissions: Res<'_, PermissionStorage>,
) {
    let (uuid, group) = match query.get(trigger.target()) {
        Ok(data) => data,
        Err(e) => {
            error!("failed to store permissions: query failed: {e}");
            return;
        }
    };

    permissions.set(**uuid, *group).unwrap();
}

fn initialize_commands(
    trigger: Trigger<'_, OnInsert, Group>,
    query: Query<'_, '_, &ConnectionId>,
    compose: Res<'_, Compose>,
    world: &World,
) {
    let cmd_pkt = get_command_packet(world, Some(trigger.target()));
    let Ok(&connection_id) = query.get(trigger.target()) else {
        error!("failed to initialize commands: player is missing ConnectionId");
        return;
    };
    compose.unicast(&cmd_pkt, connection_id).unwrap();
}

impl Plugin for PermissionPlugin {
    fn build(&self, app: &mut App) {
        let storage = storage::PermissionStorage::new(app.world().resource::<LocalDb>()).unwrap();
        app.insert_resource(storage);
        app.add_observer(load_permissions);
        app.add_observer(store_permissions);
        app.add_observer(initialize_commands);
    }
}
