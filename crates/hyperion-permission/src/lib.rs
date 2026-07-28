use clap::ValueEnum;
use flecs_ecs::{
    core::{EntityViewGet, QueryBuilderImpl, SystemAPI, World, WorldGet, id},
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

pub mod seed;
mod storage;

pub use seed::SeededGroups;

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

impl Group {
    /// Whether a player in this group may use something declared for
    /// `required`.
    ///
    /// The groups are ordinal and a higher one subsumes a lower one, with one
    /// exception: `Banned` is not the weakest group that everybody outranks,
    /// it is a specific group, and a thing declared for it is for banned
    /// players and nobody else.
    ///
    /// It lives here rather than in the code `CommandPermission` generates
    /// because a comparison written into a macro's output is a comparison
    /// nobody can put a test on.
    #[must_use]
    pub const fn allows(self, required: Self) -> bool {
        if matches!(required, Self::Banned) {
            matches!(self, Self::Banned)
        } else {
            self as u32 >= required as u32
        }
    }
}

impl Module for PermissionModule {
    fn module(world: &World) {
        world.component::<Group>();
        world
            .component::<storage::PermissionStorage>()
            .add_trait::<flecs::Singleton>();
        world
            .component::<SeededGroups>()
            .add_trait::<flecs::Singleton>();

        // Read once, at startup, and loudly. A server whose operator list is a
        // typo has nobody in charge, and the only moment that is cheap to
        // notice is before it is listening. See `seed`.
        let seeded = SeededGroups::from_env()
            .unwrap_or_else(|error| panic!("{:#}", error.context("reading the operator list")));
        tracing::info!(
            "{} account(s) hold a group from {}",
            seeded.len(),
            seed::ENV_VAR
        );
        world.set(seeded);

        world.get::<&LocalDb>(|db| {
            let storage = storage::PermissionStorage::new(db).unwrap();
            world.set(storage);
        });

        observer!(
            world,
            flecs::OnSet,
            &Uuid,
            &storage::PermissionStorage,
            &SeededGroups
        )
        .with(id::<Player>())
        .each_entity(|entity, (uuid, permissions, seeded)| {
            // Configuration outranks the database, so an operator joins in the
            // group they are configured to hold whatever `/perms set` last did
            // to them.
            let group = seeded
                .get(**uuid)
                .unwrap_or_else(|| permissions.get(**uuid));
            entity.set(group);
        });

        observer!(
            world,
            flecs::OnRemove,
            &Uuid,
            &Group,
            &storage::PermissionStorage,
            &SeededGroups
        )
        .with(id::<Player>())
        .each(|(uuid, group, permissions, seeded)| {
            // And is authoritative in the other direction too: a group that
            // came from configuration is never written to the database, so
            // deleting the line takes the group away rather than leaving it
            // behind in a file nobody remembers writing.
            if seeded.contains(**uuid) {
                return;
            }
            permissions.set(**uuid, *group).unwrap();
        });

        observer!(world, flecs::OnSet, &Group)
            // A permission group is set during login, before the client has
            // acknowledged the handover into play. The command tree is a play
            // packet, and a client reads whatever arrives in the configuration
            // state as a configuration packet, so sending it early does not
            // arrive early -- it corrupts the handover. Players already in play
            // still get the tree when their group changes.
            .with_enum(hyperion::simulation::PacketState::Play)
            .each_iter(|it, row, _group| {
                let world = it.world();
                let entity = it.entity(row);

                let root_command = hyperion::simulation::command::get_root_command_entity();

                let cmd_pkt = get_command_packet(&world, root_command, Some(*entity));

                entity.get::<&ConnectionId>(|stream| {
                    world.get::<&Compose>(|compose| {
                        compose.unicast(&cmd_pkt, *stream).unwrap();
                    });
                });
            });
    }
}
