use std::{borrow::Borrow, collections::HashMap, hash::Hash, sync::Arc};

use bow::BowCharging;
use bytemuck::{Pod, Zeroable};
use derive_more::{Constructor, Deref, DerefMut, Display, From};
use flecs_ecs::prelude::*;
use geometry::aabb::Aabb;
use glam::{I16Vec2, IVec3, Quat, Vec3};
use hyperion_utils::EntityExt;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use skin::PlayerSkin;
use tracing::{debug, error};
use uuid;
use valence_generated::block::BlockState;
use valence_protocol::{ByteAngle, VarInt, packets::play};

use crate::{
    Global,
    net::{Compose, DataBundle},
    simulation::{
        command::Command,
        entity_kind::EntityKind,
        metadata::{Metadata, MetadataPrefabs, entity::EntityFlags},
    },
    storage::ThreadLocalVec,
};

pub mod animation;
pub mod blocks;
pub mod bow;
pub mod command;
pub mod entity_kind;
pub mod event;
pub mod handlers;
pub mod inventory;
pub mod metadata;
pub mod skin;
pub mod util;

#[derive(Component, Default, Debug, Deref, DerefMut)]
pub struct StreamLookup {
    /// The UUID of all players
    inner: FxHashMap<u64, Entity>,
}

#[derive(Component, Default, Debug, Deref, DerefMut)]
pub struct PlayerUuidLookup {
    /// The UUID of all players
    inner: HashMap<Uuid, Entity>,
}

/// Communicates with the proxy server.
#[derive(Component, Deref, DerefMut, From)]
pub struct EgressComm {
    tx: tokio::sync::mpsc::UnboundedSender<bytes::Bytes>,
}

#[derive(Debug)]
pub struct DeferredMap<K, V> {
    to_add: ThreadLocalVec<(K, V)>,
    to_remove: ThreadLocalVec<K>,
    map: FxHashMap<K, V>,
}

impl<K, V> Default for DeferredMap<K, V> {
    fn default() -> Self {
        Self {
            to_add: ThreadLocalVec::default(),
            to_remove: ThreadLocalVec::default(),
            map: HashMap::default(),
        }
    }
}

impl<K: Eq + Hash, V> DeferredMap<K, V> {
    pub fn insert(&self, key: K, value: V, world: &World) {
        self.to_add.push((key, value), world);
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get(key)
    }

    pub fn remove(&self, key: K, world: &World) {
        self.to_remove.push(key, world);
    }
}

impl<K: Eq + Hash, V> DeferredMap<K, V> {
    pub fn update(&mut self) {
        for (key, value) in self.to_add.drain() {
            self.map.insert(key, value);
        }

        for key in self.to_remove.drain() {
            self.map.remove(&key);
        }
    }
}

/// The in-game name of a player.
/// todo: fix the meta
#[derive(Component, Deref, From, Display, Debug)]
#[meta]
pub struct Name(Arc<str>);

#[derive(Component, Deref, DerefMut, From, Debug, Default)]
pub struct IgnMap(DeferredMap<Arc<str>, Entity>);

#[derive(Component, Debug, Default)]
pub struct RaycastTravel;

/// A component that represents a Player. In the future, this should be broken up into multiple components.
///
/// Why should it be broken up? The more things are broken up, the more we can take advantage of Rust borrowing rules.
#[derive(Component, Debug, Default)]
pub struct Player;

/// The state of the login process.
#[derive(Component, Debug, Eq, PartialEq)]
#[repr(C)]
pub enum PacketState {
    Handshake,
    Status,
    Login,
    Play,
    Terminate,
}

#[derive(
    Component, Debug, Deref, DerefMut, PartialEq, Eq, PartialOrd, Copy, Clone, Default, Pod,
    Zeroable, From
)]
#[meta]
#[repr(C)]
pub struct Xp {
    pub amount: u16,
}

pub struct XpVisual {
    pub level: u8,
    pub prop: f32,
}

impl Xp {
    #[must_use]
    pub fn get_visual(&self) -> XpVisual {
        let level = match self.amount {
            0..=6 => 0,
            7..=15 => 1,
            16..=26 => 2,
            27..=39 => 3,
            40..=54 => 4,
            55..=71 => 5,
            72..=90 => 6,
            91..=111 => 7,
            112..=134 => 8,
            135..=159 => 9,
            160..=186 => 10,
            187..=215 => 11,
            216..=246 => 12,
            247..=279 => 13,
            280..=314 => 14,
            315..=351 => 15,
            352..=393 => 16,
            394..=440 => 17,
            441..=492 => 18,
            493..=549 => 19,
            550..=611 => 20,
            612..=678 => 21,
            679..=750 => 22,
            751..=827 => 23,
            828..=909 => 24,
            910..=996 => 25,
            997..=1088 => 26,
            1089..=1185 => 27,
            1186..=1287 => 28,
            1288..=1394 => 29,
            1395..=1506 => 30,
            1507..=1627 => 31,
            1628..=1757 => 32,
            1758..=1896 => 33,
            1897..=2044 => 34,
            2045..=2201 => 35,
            2202..=2367 => 36,
            2368..=2542 => 37,
            2543..=2726 => 38,
            2727..=2919 => 39,
            2920..=3121 => 40,
            3122..=3332 => 41,
            3333..=3552 => 42,
            3553..=3781 => 43,
            3782..=4019 => 44,
            4020..=4266 => 45,
            4267..=4522 => 46,
            4523..=4787 => 47,
            4788..=5061 => 48,
            5062..=5344 => 49,
            5345..=5636 => 50,
            5637..=5937 => 51,
            5938..=6247 => 52,
            6248..=6566 => 53,
            6567..=6894 => 54,
            6895..=7231 => 55,
            7232..=7577 => 56,
            7578..=7932 => 57,
            7933..=8296 => 58,
            8297..=8669 => 59,
            8670..=9051 => 60,
            9052..=9442 => 61,
            9443..=9842 => 62,
            _ => 63,
        };

        let (level_start, next_level_start) = match level {
            0 => (0, 7),
            1 => (7, 16),
            2 => (16, 27),
            3 => (27, 40),
            4 => (40, 55),
            5 => (55, 72),
            6 => (72, 91),
            7 => (91, 112),
            8 => (112, 135),
            9 => (135, 160),
            10 => (160, 187),
            11 => (187, 216),
            12 => (216, 247),
            13 => (247, 280),
            14 => (280, 315),
            15 => (315, 352),
            16 => (352, 394),
            17 => (394, 441),
            18 => (441, 493),
            19 => (493, 550),
            20 => (550, 612),
            21 => (612, 679),
            22 => (679, 751),
            23 => (751, 828),
            24 => (828, 910),
            25 => (910, 997),
            26 => (997, 1089),
            27 => (1089, 1186),
            28 => (1186, 1288),
            29 => (1288, 1395),
            30 => (1395, 1507),
            31 => (1507, 1628),
            32 => (1628, 1758),
            33 => (1758, 1897),
            34 => (1897, 2045),
            35 => (2045, 2202),
            36 => (2202, 2368),
            37 => (2368, 2543),
            38 => (2543, 2727),
            39 => (2727, 2920),
            40 => (2920, 3122),
            41 => (3122, 3333),
            42 => (3333, 3553),
            43 => (3553, 3782),
            44 => (3782, 4020),
            45 => (4020, 4267),
            46 => (4267, 4523),
            47 => (4523, 4788),
            48 => (4788, 5062),
            49 => (5062, 5345),
            50 => (5345, 5637),
            51 => (5637, 5938),
            52 => (5938, 6248),
            53 => (6248, 6567),
            54 => (6567, 6895),
            55 => (6895, 7232),
            56 => (7232, 7578),
            57 => (7578, 7933),
            58 => (7933, 8297),
            59 => (8297, 8670),
            60 => (8670, 9052),
            61 => (9052, 9443),
            62 => (9443, 9843),
            _ => (9843, 10242), // Extrapolated next value
        };

        let prop = f32::from(self.amount - level_start) / f32::from(next_level_start - level_start);

        XpVisual { level, prop }
    }
}

pub const FULL_HEALTH: f32 = 20.0;

#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct ConfirmBlockSequences(pub Vec<i32>);

#[derive(Component, Debug, Eq, PartialEq, Default)]
#[expect(missing_docs)]
#[meta]
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

/// If the entity can be targeted by non-player entities.
#[derive(Component)]
pub struct AiTargetable;

/// The full pose of an entity. This is used for both [`Player`] and [`Npc`].
#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Serialize,
    Deserialize,
    Deref,
    DerefMut,
    From,
    PartialEq
)]
#[meta]
pub struct Position {
    /// The (x, y, z) position of the entity.
    /// Note we are using [`Vec3`] instead of [`glam::DVec3`] because *cache locality* is important.
    /// However, the Notchian server uses double precision floating point numbers for the position.
    position: Vec3,
}

impl Position {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: Vec3::new(x, y, z),
        }
    }
}

#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Deref,
    DerefMut,
    Default,
    Constructor,
    PartialEq
)]
#[meta]
pub struct Yaw {
    yaw: f32,
}

impl Display for Yaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let yaw = self.yaw;
        write!(f, "{yaw}")
    }
}

impl Display for Pitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pitch = self.pitch;
        write!(f, "{pitch}")
    }
}

#[derive(
    Component,
    Copy,
    Clone,
    Debug,
    Deref,
    DerefMut,
    Default,
    Constructor,
    PartialEq
)]
#[meta]
pub struct Pitch {
    pitch: f32,
}

const PLAYER_WIDTH: f32 = 0.6;
const PLAYER_HEIGHT: f32 = 1.8;

#[derive(Component, Copy, Clone, Debug, Constructor, PartialEq)]
#[meta]
pub struct EntitySize {
    pub half_width: f32,
    pub height: f32,
}

impl Display for EntitySize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let half_width = self.half_width;
        let height = self.height;
        write!(f, "{half_width}x{height}")
    }
}

impl Default for EntitySize {
    fn default() -> Self {
        Self {
            half_width: PLAYER_WIDTH / 2.0,
            height: PLAYER_HEIGHT,
        }
    }
}

impl Position {
    #[must_use]
    pub fn sound_position(&self) -> IVec3 {
        let position = self.position * 8.0;
        position.as_ivec3()
    }
}

#[derive(Component, Debug, Copy, Clone)]
#[meta]
pub struct ChunkPosition {
    pub position: I16Vec2,
}

const SANE_MAX_RADIUS: i16 = 128;

impl ChunkPosition {
    #[must_use]
    #[expect(missing_docs)]
    pub const fn null() -> Self {
        // todo: huh
        Self {
            position: I16Vec2::new(SANE_MAX_RADIUS, SANE_MAX_RADIUS),
        }
    }
}

#[must_use]
pub fn aabb(position: Vec3, size: EntitySize) -> Aabb {
    let half_width = size.half_width;
    let height = size.height;
    Aabb::new(
        position - Vec3::new(half_width, 0.0, half_width),
        position + Vec3::new(half_width, height, half_width),
    )
}

#[must_use]
pub fn block_bounds(position: Vec3, size: EntitySize) -> (IVec3, IVec3) {
    let bounding = aabb(position, size);
    let min = bounding.min.floor().as_ivec3();
    let max = bounding.max.ceil().as_ivec3();

    (min, max)
}

/// The initial player spawn position. todo: this should not be a constant
pub const PLAYER_SPAWN_POSITION: Vec3 = Vec3::new(-8_526_209_f32, 100f32, -6_028_464f32);

impl Position {
    /// Get the chunk position of the center of the player's bounding box.
    #[must_use]
    #[expect(clippy::cast_possible_truncation)]
    pub fn to_chunk(&self) -> I16Vec2 {
        let x = self.x as i32;
        let z = self.z as i32;
        let x = x >> 4;
        let z = z >> 4;

        let x = i16::try_from(x).unwrap();
        let z = i16::try_from(z).unwrap();

        I16Vec2::new(x, z)
    }
}

/// The reaction of an entity, in particular to collisions as calculated in `entity_detect_collisions`.
///
/// Why is this useful?
///
/// - We want to be able to detect collisions in parallel.
/// - Since we are accessing bounding boxes in parallel,
///   we need to be able to make sure the bounding boxes are immutable (unless we have something like a
///   [`std::sync::Arc`] or [`std::sync::RwLock`], but this is not efficient).
/// - Therefore, we have an [`Velocity`] component which is used to store the reaction of an entity to collisions.
/// - Later we can apply the reaction to the entity's [`Position`] to move the entity.
#[derive(Component, Default, Debug, Copy, Clone, PartialEq)]
#[meta]
pub struct Velocity(pub Vec3);

impl Velocity {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self(Vec3::new(x, y, z))
    }

    #[must_use]
    pub fn to_packet_units(self) -> valence_protocol::Velocity {
        valence_protocol::Velocity::from_ms_f32((self.0 * 20.0).into())
    }
}

#[derive(Component)]
pub struct SimModule;

impl Module for SimModule {
    fn module(world: &World) {
        component!(world, VarInt).member::<i32>("x");

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
            .member::<f32>("x")
            .member::<f32>("y")
            .member::<f32>("z")
            .member::<f32>("w");

        component!(world, BlockState).member::<u16>("id");

        world.component::<Velocity>().meta();
        world.component::<Player>();
        world.component::<Visible>();
        world.component::<Spawn>();

        world.component::<EntityKind>().meta();

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
        world.component::<Command>();

        component!(world, IgnMap);

        world.component::<Position>().meta();

        world.component::<Name>();
        component!(world, Name).opaque_func(meta_ser_stringify_type_display::<Name>);

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

        world.component::<BowCharging>();
        component!(world, BowCharging).opaque_func(meta_ser_stringify_type_display::<BowCharging>);

        observer!(
            world,
            Spawn,
            &Compose($),
            [filter] & Uuid,
            [filter] & Position,
            [filter] & Pitch,
            [filter] & Yaw,
            [filter] & Velocity,
        )
        .with::<flecs::Any>()
        .with_enum_wildcard::<EntityKind>()
        .each_iter(|it, row, (compose, uuid, position, pitch, yaw, velocity)| {
            let system = it.system();

            let entity = it.entity(row);
            let minecraft_id = entity.minecraft_id();

            let mut bundle = DataBundle::new(compose, system);

            let mut spawn_entity = move |kind: EntityKind| -> anyhow::Result<()> {
                let kind = kind as i32;

                let velocity = velocity.to_packet_units();

                let packet = play::EntitySpawnS2c {
                    entity_id: VarInt(minecraft_id),
                    object_uuid: uuid.0,
                    kind: VarInt(kind),
                    position: position.as_dvec3(),
                    pitch: ByteAngle::from_degrees(**pitch),
                    yaw: ByteAngle::from_degrees(**yaw),
                    head_yaw: ByteAngle::from_degrees(0.0), // todo:
                    data: VarInt::default(),                // todo:
                    velocity,
                };

                bundle.add_packet(&packet).unwrap();

                let packet = play::EntityVelocityUpdateS2c {
                    entity_id: VarInt(minecraft_id),
                    velocity,
                };

                bundle.add_packet(&packet).unwrap();

                bundle.broadcast_local(position.to_chunk()).unwrap();

                Ok(())
            };

            debug!("spawned entity");

            entity.get::<&EntityKind>(|kind| {
                if let Err(e) = spawn_entity(*kind) {
                    error!("failed to spawn entity: {e}");
                }
            });
        });

        // for every new entity without a UUID, give it one
        world
            .observer::<flecs::OnAdd, ()>()
            .with_enum_wildcard::<EntityKind>()
            .without::<Uuid>()
            .each_entity(|entity, ()| {
                debug!("adding uuid to entity");
                let uuid = uuid::Uuid::new_v4();
                entity.set(Uuid::from(uuid));
            });

        world
            .observer::<flecs::OnSet, ()>()
            .with_enum_wildcard::<EntityKind>()
            .each_entity(move |entity, ()| {
                entity.get::<&EntityKind>(|kind| match kind {
                    EntityKind::BlockDisplay => {
                        entity.is_a_id(prefabs.block_display_base);
                    }
                    EntityKind::Player => {
                        entity.is_a_id(prefabs.player_base);
                    }
                    _ => {}
                });
            });
    }
}

#[derive(Component)]
pub struct Spawn;

#[derive(Component)]
pub struct Visible;

#[must_use]
pub fn get_rotation_from_velocity(velocity: Vec3) -> (f32, f32) {
    let yaw = (-velocity.x).atan2(velocity.z).to_degrees().abs(); // Correct yaw calculation
    let pitch = (-velocity.y).atan2(velocity.length()).to_degrees(); // Correct pitch calculation
    (yaw, pitch)
}

#[must_use]
pub fn get_direction_from_rotation(yaw: f32, pitch: f32) -> Vec3 {
    // Convert angles from degrees to radians
    let yaw_rad = yaw.to_radians();
    let pitch_rad = pitch.to_radians();

    Vec3::new(
        -pitch_rad.cos() * yaw_rad.sin(), // x = -cos(pitch) * sin(yaw)
        -pitch_rad.sin(),                 // y = -sin(pitch)
        pitch_rad.cos() * yaw_rad.cos(),  // z = cos(pitch) * cos(yaw)
    )
}
