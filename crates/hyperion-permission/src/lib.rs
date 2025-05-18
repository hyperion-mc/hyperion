use clap::ValueEnum;
use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, SystemAPI, TermBuilderImpl, World, WorldGet, id},
    macros::{Component, observer},
    prelude::{Module, flecs},
};
use hyperion::{
    net::{Compose, ConnectionId},
    simulation::{Player, Uuid, command::get_command_packet},
    storage::LocalDb,
};
use num_derive::{FromPrimitive, ToPrimitive};

#[derive(Component)]
pub struct PermissionModule;

mod storage;

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

impl Module for PermissionModule {
    fn module(world: &World) {
        world.component::<Group>();
        world.component::<storage::PermissionStorage>();

        world.get::<&LocalDb>(|db| {
            let storage = storage::PermissionStorage::new(db).unwrap();
            world.set(storage);
        });

        observer!(world, flecs::OnSet, &Uuid, &storage::PermissionStorage($))
            .with(id::<Player>())
            .each_entity(|entity, (uuid, permissions)| {
                let group = permissions.get(**uuid);
                entity.set(group);
            });

        observer!(world, flecs::OnRemove, &Uuid, &Group, &storage::PermissionStorage($))
            .with(id::<Player>())
            .each(|(uuid, group, permissions)| {
                permissions.set(**uuid, *group).unwrap();
            });

        observer!(world, flecs::OnSet, &Group).each_iter(|it, row, _group| {
            let system = it.system();
            let world = it.world();
            let entity = it.entity(row).expect("row must be in bounds");

            let root_command = hyperion::simulation::command::get_root_command_entity();

            let cmd_pkt = get_command_packet(&world, root_command, Some(*entity));

            entity.get::<&ConnectionId>(|stream| {
                world.get::<&Compose>(|compose| {
                    compose.unicast(&cmd_pkt, *stream, system).unwrap();
                });
            });
        });
    }
}
