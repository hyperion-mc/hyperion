//! The game world: what exists in it, and what the rules do to it each tick.
//!
//! This module is wiring. [`SimModule`] registers every component the
//! simulation owns and every observer that reacts to one changing, and that
//! registration order is load-bearing rather than incidental. What the
//! components mean lives in the submodules, and the ones declared here are
//! markers with no behaviour of their own.
//!
//! The split runs by subject, not by size:
//!
//! - [`pose`] is where an entity is and how big it is, which nearly every
//!   other system reads and almost none of them write.
//! - [`blocks`] is the world itself, loaded and streamed by chunk.
//! - [`handlers`] and [`packet`] are the inbound edge, turning what a client
//!   sent into events other modules can watch.
//! - [`event`] is that vocabulary of events.
//! - [`lookup`] finds an entity again from an id a packet carried.
//! - [`metadata`] is the per-entity-kind state the client has to be told.
//! - [`command`], [`inventory`], [`flight`], [`gamemode`], [`animation`],
//!   [`skin`], [`xp`] are one game concept each.
//!
//! Nothing here knows which game is being played. That is `events/`.

use std::sync::Arc;

use derive_more::{Deref, DerefMut, Display, From};
use flecs_ecs::prelude::*;
use glam::{IVec3, Quat, Vec3};
use hyperion_minecraft_proto::{
    Uuid as ProtoUuid,
    generated::packet_id::play::clientbound::PacketId,
    packets::{
        play::entity::{AddEntity, pack_degrees},
        play_login::{PlayerPosition, PositionMoveRotation, Relative, Vec3 as LoginVec3},
    },
    types::Vec3 as ProtoVec3,
};
use hyperion_utils::EntityExt;
use skin::PlayerSkin;
use tracing::{debug, error};
use uuid;
use valence_generated::block::BlockState;
use valence_protocol::VarInt;

use crate::{
    Global,
    net::{
        Compose, ConnectionId, DataBundle,
        protocol::{Clientbound, send},
    },
    simulation::{
        command::Command,
        entity_kind::EntityKind,
        metadata::{Metadata, MetadataPrefabs, entity::EntityFlags},
    },
};

pub mod animation;
pub mod blocks;
pub mod command;
pub mod entity_kind;
pub mod event;
pub mod flight;
pub mod gamemode;
pub mod handlers;
pub mod inventory;
pub mod lookup;
pub mod metadata;
pub mod packet;
pub mod pose;
pub mod projectile_motion;
pub mod skin;
pub mod util;
pub mod world_time;
pub mod xp;

// Re-exported at `simulation::` rather than left behind their new module
// names. Which file a component is declared in is an implementation detail;
// the path a game module imports it by is not, and every one of these was
// public directly here before the split.
pub use flight::{Flight, FlyingSpeed, MovementTracking};
pub use lookup::{DeferredMap, IgnMap, PlayerUuidLookup, StreamLookup};
pub use pose::{
    ChunkPosition, EntitySize, PLAYER_SPAWN_POSITION, PendingTeleportation, Pitch, Position,
    Velocity, Yaw, aabb, block_bounds, get_direction_from_rotation,
};
pub use world_time::{WorldTime, WorldTimeModule};
pub use xp::{Xp, XpVisual};

/// Communicates with the proxy server.
#[derive(Component, Clone, Deref, DerefMut, From)]
pub struct EgressComm {
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<bytes::Bytes>,
}

/// The in-game name of a player.
// No #[flecs(meta)]: registered below as opaque, and a type cannot be both.
#[derive(Component, Deref, From, Display, Debug)]
pub struct Name(Arc<str>);

#[derive(Component, Debug, Default)]
pub struct RaycastTravel;

/// A component that represents a Player. In the future, this should be broken up into multiple components.
///
/// Why should it be broken up? The more things are broken up, the more we can take advantage of Rust borrowing rules.
#[derive(Component, Debug, Default)]
pub struct Player;

/// The state of the login process.
///
/// `Configuration` is the state 1.20.2 introduced between login and play. It
/// is unreachable on protocol 763, which predates it, and exists here
/// unconditionally only because a flecs enum component's variants are part of
/// its registered type rather than something a cargo feature can vary.
#[derive(Component, Debug, Eq, PartialEq)]
#[repr(C)]
pub enum PacketState {
    Handshake,
    Status,
    Login,
    Play,
    Terminate,
    Configuration,
}

pub const FULL_HEALTH: f32 = 20.0;

#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct ConfirmBlockSequences(pub Vec<i32>);
// Nothing drains this and nothing turns it into a `BlockChangedAck`, so a
// handler pushing here has acknowledged nothing. `Blocks::to_confirm` is the
// queue the egress pass actually reads. Left in place rather than deleted
// because removing it is wider than the change this comment came from, but it
// has already misled one reader into shipping a refusal the client never heard
// about. See ENG-10806.

#[derive(Component, Debug, Eq, PartialEq, Default)]
#[expect(missing_docs)]
#[flecs(meta)]
pub struct ImmuneStatus {
    /// The tick until the player is immune to player attacks.
    pub until: i64,
}

impl ImmuneStatus {
    #[must_use]
    #[expect(missing_docs)]
    pub const fn is_invincible(&self, global: &Global) -> bool {
        global.tick < self.until
    }
}

/// Communication struct. Maybe should be refactored to some extent.
/// This to communicate back from an [`crate::runtime::AsyncRuntime`] to the main thread.
#[derive(Component)]
pub struct Comms {
    /// Skin rx channel.
    pub skins_rx: kanal::Receiver<(Entity, PlayerSkin)>,
    /// Skin tx channel.
    pub skins_tx: kanal::Sender<(Entity, PlayerSkin)>,
}

impl Default for Comms {
    fn default() -> Self {
        let (skins_tx, skins_rx) = kanal::unbounded();

        Self { skins_rx, skins_tx }
    }
}

/// A UUID component. Generally speaking, this tends to be tied to entities with a [`Player`] component.
#[derive(
    Component, Copy, Clone, Debug, Deref, From, Hash, Eq, PartialEq, Display
)]
pub struct Uuid(pub uuid::Uuid);

impl Uuid {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

/// Any living minecraft entity that is NOT a player.
///
/// Example: zombie, skeleton, etc.
#[derive(Component, Debug)]
pub struct Npc;

/// The running multiplier of the entity. This defaults to 1.0.
#[derive(Component, Debug, Copy, Clone)]
pub struct RunningSpeed(pub f32);

impl Default for RunningSpeed {
    fn default() -> Self {
        Self(0.1)
    }
}

#[derive(Component)]
pub struct Owner {
    pub entity: Entity,
}

impl Owner {
    #[must_use]
    pub const fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

/// If the entity can be targeted by non-player entities.
#[derive(Component)]
pub struct AiTargetable;

/// The `add_entity` that puts this entity in a client's world.
///
/// One packet rather than the spawn-plus-velocity pair 1.20.1 needed: since
/// 26.2 `add_entity` carries the velocity itself, through the same packed
/// `Vec3.LP_STREAM_CODEC` that `set_entity_motion` uses.
pub(crate) fn add_entity(
    minecraft_id: i32,
    kind: EntityKind,
    uuid: &Uuid,
    position: &Position,
    pitch: Pitch,
    yaw: Yaw,
    velocity: &Velocity,
) -> AddEntity {
    let location = position.as_dvec3();

    AddEntity {
        id: minecraft_id,
        uuid: ProtoUuid(uuid.0.as_u128()),
        r#type: kind.expect_entity_type().id(),
        x: location.x,
        y: location.y,
        z: location.z,
        // Blocks per tick, which is what `Velocity` already holds.
        movement: ProtoVec3 {
            x: f64::from(velocity.0.x),
            y: f64::from(velocity.0.y),
            z: f64::from(velocity.0.z),
        },
        x_rot: pack_degrees(*pitch),
        y_rot: pack_degrees(*yaw),
        // The client tracks head yaw separately rather than deriving it from
        // the body, so a zero here spawns everything facing north.
        y_head_rot: pack_degrees(*yaw),
        // No entity this server spawns defines type-specific spawn data.
        data: 0,
    }
}

#[derive(Component)]
pub struct SimModule;

impl Module for SimModule {
    // Read as a table of contents. Each step runs in the order flecs needs:
    // a relation has to be a registered entity before anything points at it,
    // and an observer has to come after the components it watches.
    fn module(world: &World) {
        register_reflection(world);
        let prefabs = register_components(world);
        register_observers(world, prefabs);
    }
}

/// The metadata flecs cannot derive on its own.
///
/// `#[flecs(meta)]` covers hyperion's own structs. These are the ones it
/// cannot reach: types owned by glam and valence, plus [`EntitySize`], which
/// registers as an opaque serialised through `Display` because flecs aborts if
/// a type is registered as both a struct and an opaque.
fn register_reflection(world: &World) {
    component!(world, VarInt).member(id::<i32>(), "x");

    component!(world, EntitySize).opaque_func(meta_ser_stringify_type_display::<EntitySize>);

    component!(world, IVec3 {
        x: i32,
        y: i32,
        z: i32
    });
    component!(world, Vec3 {
        x: f32,
        y: f32,
        z: f32
    });

    component!(world, Quat)
        .member(id::<f32>(), "x")
        .member(id::<f32>(), "y")
        .member(id::<f32>(), "z")
        .member(id::<f32>(), "w");

    component!(world, BlockState).member(id::<u16>(), "id");
}

/// Every component and singleton the simulation owns.
///
/// Returns the metadata prefabs because [`register_observers`] needs them to
/// pick a base for each entity kind it spawns. They used to reach it as a
/// closure capture over one long function body; threading it through says so.
fn register_components(world: &World) -> MetadataPrefabs {
    world.component::<Velocity>().meta();
    world.component::<Player>();
    world.component::<Visible>();
    world.component::<Spawn>();
    world.component::<BroadcastProjectile>();
    world.component::<Owner>();
    world.component::<PendingTeleportation>();
    world.component::<FlyingSpeed>();
    world.component::<MovementTracking>();
    world.component::<Flight>().meta();

    world.component::<EntityKind>().meta();
    world.component::<projectile_motion::ProjectileMotion>();

    // todo: how
    // world
    //     .component::<EntityKind>()
    //     .add_trait::<(flecs::With, Yaw)>()
    //     .add_trait::<(flecs::With, Pitch)>()
    //     .add_trait::<(flecs::With, Velocity)>();

    world.component::<MetadataPrefabs>();
    world.component::<EntityFlags>();
    let prefabs = metadata::register_prefabs(world);

    world.set(prefabs);

    world.component::<Xp>().meta();

    world.component::<PlayerSkin>();
    world.component::<gamemode::Gamemode>();
    world
        .component::<gamemode::DefaultGamemode>()
        .add_trait::<flecs::Singleton>();
    world.set(gamemode::DefaultGamemode::default());
    world.component::<Command>();
    // The completion vocabulary. `Suggests` is a relation, so it has to be
    // a registered entity before an argument node can point at anything
    // with it, and `flecs_manual_registration` means that will not happen
    // on its own.
    world.component::<command::Suggests>();
    world.component::<command::SuggestionLabel>();
    world.component::<command::FixedSuggestions>();

    component!(world, IgnMap);

    world.component::<Position>().meta();

    world.component::<Name>();
    component!(world, Name).opaque_func(meta_ser_stringify_type_display::<Name>);

    // A command argument declared with `.completes("player", Player::id())`
    // offers whoever is connected at the moment the player presses tab.
    // Here rather than in a game module because both halves are hyperion's:
    // it owns `Player` and it owns the `Name` that login puts on one, so
    // every event wants the same answer and none of them should restate it.
    world
        .component::<Player>()
        .set(command::SuggestionLabel(|player| {
            player.try_get::<&Name>(std::string::ToString::to_string)
        }));

    world.component::<AiTargetable>();
    world.component::<ImmuneStatus>().meta();

    world.component::<Uuid>();
    component!(world, Uuid).opaque_func(meta_ser_stringify_type_display::<Uuid>);

    world.component::<ChunkPosition>().meta();
    world.component::<ConfirmBlockSequences>();
    world.component::<animation::ActiveAnimation>();

    world.component::<hyperion_inventory::PlayerInventory>();
    world.component::<hyperion_inventory::CursorItem>();

    world
        .component::<Player>()
        .add_trait::<(flecs::With, hyperion_inventory::CursorItem)>();

    prefabs
}

/// The observers that react to those components changing.
fn register_observers(world: &World, prefabs: MetadataPrefabs) {
    observer!(
        world,
        Spawn,
        &Compose,
        [filter] & Uuid,
        [filter] & Position,
        [filter] & Pitch,
        [filter] & Yaw,
        [filter] & Velocity,
    )
    .with(id::<flecs::Any>())
    .with_enum_wildcard::<EntityKind>()
    .each_iter(|it, row, (compose, uuid, position, pitch, yaw, velocity)| {
        let entity = it.entity(row);
        let minecraft_id = entity.minecraft_id();

        let mut bundle = DataBundle::new(compose);

        let mut spawn_entity = move |kind: EntityKind| -> anyhow::Result<()> {
            let packet = add_entity(minecraft_id, kind, uuid, position, *pitch, *yaw, velocity);

            bundle.add_packet(Clientbound::new(PacketId::AddEntity.to_raw(), &packet))?;
            bundle.broadcast_local(position.to_chunk())?;

            Ok(())
        };

        debug!("spawned entity");

        entity.get::<&EntityKind>(|kind| {
            if let Err(e) = spawn_entity(*kind) {
                error!("failed to spawn entity: {e}");
            }
        });
    });

    // Anything the server spawns needs an id invented for it.
    //
    // A player is the exception, and `ConnectionId` is what excludes them:
    // their id was chosen and sent in `LoginFinished` before the entity had
    // an `EntityKind` at all, and this observer would otherwise overwrite
    // it. Not `without(Uuid)` alone, which looks like it covers this and
    // does not: the login handler builds the player from inside a system,
    // so its `set` is queued, this observer fires while an earlier queued
    // command merges and sees no `Uuid` yet, and its own `set` is appended
    // behind the login handler's and lands last. The player ended up
    // wearing an id their own client had never been told. See ENG-10813.
    world
        .observer_named::<flecs::OnAdd, ()>("assign_entity_uuid")
        .with_enum_wildcard::<EntityKind>()
        .without(id::<Uuid>())
        .without(id::<crate::net::ConnectionId>())
        .each_entity(|entity, ()| {
            debug!("adding uuid to entity");
            entity.set(Uuid::new_v4());
        });

    // Seed a projectile's per-tick motion from its kind's vanilla default the
    // moment the kind is set, so the integrator reads a component a game module
    // can also override per instance (a hook with no gravity, a heavier lob). A
    // kind with no entry in `SIMULATED` gets nothing, exactly as before.
    world
        .observer_named::<flecs::OnAdd, ()>("seed_projectile_motion")
        .with_enum_wildcard::<EntityKind>()
        .each_entity(|entity, ()| {
            if let Some(motion) = entity
                .try_get::<&EntityKind>(|kind| kind.projectile_motion())
                .flatten()
            {
                entity.set(motion);
            }
        });

    world
        .observer_named::<flecs::OnSet, ()>("apply_entity_prefab")
        .with_enum_wildcard::<EntityKind>()
        .each_entity(move |entity, ()| {
            entity.get::<&EntityKind>(|kind| match kind {
                EntityKind::BlockDisplay => {
                    entity.is_a(prefabs.block_display_base);
                }
                EntityKind::Item => {
                    entity.is_a(prefabs.item_base);
                }
                EntityKind::Player => {
                    entity.is_a(prefabs.player_base);
                }
                _ => {}
            });
        });

    // whenever a Player component is added, we add the Flight component to them.
    world
        .component::<Player>()
        .add_trait::<(flecs::With, Flight)>();
    world
        .component::<Player>()
        .add_trait::<(flecs::With, FlyingSpeed)>();

    observer!(
        world,
        flecs::OnSet,
        &PendingTeleportation,
        &Compose,
        &Yaw,
        &Pitch,
        &ConnectionId
    )
    .each(|(pending_teleportation, compose, yaw, pitch, connection)| {
        // The same packet the join path sends, at the same id. Sending
        // 1.20.1's `PlayerPositionLook` here instead left a 26.2 client with
        // no teleport to confirm, so `AcceptTeleportation` never came back,
        // `PendingTeleportation` was never removed, and `sync_player_entity`
        // took its pending-teleport branch forever after: no movement, no
        // rotation and no velocity ever reached anyone again. A mid-match
        // teleport is the whole of respawning.
        let destination = pending_teleportation.destination.as_dvec3();
        let pkt = PlayerPosition {
            id: pending_teleportation.teleport_id,
            change: PositionMoveRotation {
                position: LoginVec3 {
                    x: destination.x,
                    y: destination.y,
                    z: destination.z,
                },
                // A correction teleport stops the player rather than
                // carrying their velocity into the new position.
                delta_movement: LoginVec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                y_rot: **yaw,
                x_rot: **pitch,
            },
            relatives: Relative::NONE,
        };

        send(
            compose,
            *connection,
            PacketId::PlayerPosition.to_raw(),
            &pkt,
        )
        .unwrap();
    });

    observer!(
        world,
        flecs::OnSet,
        &FlyingSpeed,
        &Compose,
        &ConnectionId,
        &Flight
    )
    .each(|(flying_speed, compose, connection, flight)| {
        send(
            compose,
            *connection,
            PacketId::PlayerAbilities.to_raw(),
            &flight::abilities(*flight, *flying_speed),
        )
        .unwrap();
    });

    observer!(
        world,
        flecs::OnSet,
        &Flight,
        &Compose,
        &ConnectionId,
        &FlyingSpeed
    )
    .each(|(flight, compose, connection, flying_speed)| {
        send(
            compose,
            *connection,
            PacketId::PlayerAbilities.to_raw(),
            &flight::abilities(*flight, *flying_speed),
        )
        .unwrap();
    });
}

#[derive(Component)]
pub struct Spawn;

/// Marks a projectile whose position hyperion broadcasts to its channel every
/// tick, regardless of who owns it.
///
/// The per-tick `EntityPositionSync` used to live only inside
/// `update_projectile_positions`, which requires an [`Owner`] to integrate and
/// collide the entity. A game half that owns its own flight (smash decorates a
/// projectile it is already integrating) wants the broadcast without the
/// integration, so the two are split: this marker drives the broadcast, an
/// [`Owner`] drives the integration, and an entity can carry either or both.
#[derive(Component)]
pub struct BroadcastProjectile;

#[derive(Component)]
pub struct Visible;
