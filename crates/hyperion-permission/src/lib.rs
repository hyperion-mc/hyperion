mod storage;

use bevy::{ecs::world::OnDespawn, prelude::*};
use clap::ValueEnum;
use hyperion::{simulation::Uuid, storage::LocalDb};
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
    query: Query<'_, '_, &Uuid>,
    permissions: Res<'_, PermissionStorage>,
    mut commands: Commands<'_, '_>,
) {
    let uuid = match query.get(trigger.target()) {
        Ok(uuid) => uuid,
        Err(e) => {
            error!("failed to load permissions: query failed: {e}");
            return;
        }
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

impl Plugin for PermissionPlugin {
    fn build(&self, app: &mut App) {
        let storage = storage::PermissionStorage::new(app.world().resource::<LocalDb>()).unwrap();
        app.insert_resource(storage);
        app.add_observer(load_permissions);
        app.add_observer(store_permissions);

        // observer!(world, flecs::OnSet, &Group).each_iter(|it, row, _group| {
        //     let system = it.system();
        //     let world = it.world();
        //     let entity = it.entity(row).expect("row must be in bounds");
        //
        //     let root_command = hyperion::simulation::command::get_root_command_entity();
        //
        //     let cmd_pkt = get_command_packet(&world, root_command, Some(*entity));
        //
        //     entity.get::<&ConnectionId>(|stream| {
        //         world.get::<&Compose>(|compose| {
        //             compose.unicast(&cmd_pkt, *stream, system).unwrap();
        //         });
        //     });
        // });
    }
}
