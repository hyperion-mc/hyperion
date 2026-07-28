//! Turning map descriptions into blocks hyperion serves, and choosing which
//! arena a match is played on.
//!
//! The world is empty to begin with -- `Blocks::empty` rather than a downloaded
//! save -- and every arena plus the hub is stamped into it at boot. That is what
//! makes the geometry real: a client walks on it, projectiles would collide with
//! it, and a player who falls off it falls past the kill plane rather than
//! landing on terrain that happens to be there.
//!
//! One world, several regions. The hub sits at the origin and each arena gets
//! its own [`REGION_STRIDE`]-wide slot along +X, so map files can be written in
//! local coordinates and switching maps is a teleport rather than a world load.
//! Mineplex switched worlds instead, because a Bukkit server could; hyperion
//! serves exactly one set of chunks, so the regions have to be far enough apart
//! that no view distance reaches from one to the next.

use flecs_ecs::prelude::*;
use glam::{I16Vec2, IVec3, Vec3};
use hyperion::{
    BlockKind, BlockState,
    net::Channel as EntityChannel,
    runtime::AsyncRuntime,
    simulation::{
        Pitch, Position as MobPosition, Spawn, Uuid, Velocity, Yaw, blocks::Blocks,
        entity_kind::EntityKind,
    },
};

use crate::{
    map::{self, MapSpec, parse},
    module::{
        arena::Arena,
        lobby::{Lobby, Phase, PhaseChanged},
        player::Player,
        selector,
    },
    server::{PlayerId, ServerHandle},
};

/// Blocks between one region's centre and the next.
///
/// Has to exceed twice the largest view distance so a player in the hub is
/// never sent an arena's chunks, and the largest map's own half-width so the
/// two never overlap. 512 covers a 32-chunk view distance with room over.
pub const REGION_STRIDE: i32 = 512;

/// The hub's region index. Arenas start at 1.
const HUB_REGION: i32 = 0;

/// Where a region's local origin sits in world coordinates.
#[must_use]
pub const fn region_origin(index: i32) -> IVec3 {
    IVec3::new(index * REGION_STRIDE, 0, 0)
}

/// The maps this server rotates through, and which one is next.
#[derive(Component, Debug)]
pub struct MapRotation {
    pub maps: Vec<Loaded>,
    /// Index into `maps` of the arena the next match is played on.
    pub next: usize,
}

/// One arena, parsed and placed.
#[derive(Debug)]
pub struct Loaded {
    pub spec: MapSpec,
    pub origin: IVec3,
}

impl Loaded {
    /// The map's spawn points in world coordinates.
    #[must_use]
    pub fn spawns(&self) -> Vec<Vec3> {
        self.spec
            .spawns
            .iter()
            .map(|spawn| *spawn + self.origin.as_vec3())
            .collect()
    }

    /// The [`Arena`] this map implies. Written to the singleton before a match
    /// starts, which is what points the game's death plane and respawns at this
    /// map rather than the last one.
    #[must_use]
    pub fn arena(&self) -> Arena {
        Arena {
            name: self.spec.name,
            #[expect(
                clippy::cast_precision_loss,
                reason = "region origins are small multiples of 512"
            )]
            kill_y: self.spec.kill_y + self.origin.y as f32,
            spawns: self.spawns(),
        }
    }
}

/// Where players stand between matches. A singleton so the return-to-lobby
/// observer and the join handler agree on one answer.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Hub {
    pub spawns: Vec<Vec3>,
}

impl Hub {
    /// A standing spot, wrapping. Spreading arrivals over the ring rather than
    /// stacking them on one block is the difference between a lobby and a pile.
    #[must_use]
    pub fn spawn(&self, index: u64) -> Vec3 {
        if self.spawns.is_empty() {
            return Vec3::new(0.0, 64.0, 0.0);
        }
        let len = self.spawns.len() as u64;
        self.spawns[usize::try_from(index % len).unwrap_or(0)]
    }
}

#[derive(Component)]
pub struct MapModule;

impl Module for MapModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();

        world
            .component::<MapRotation>()
            .add_trait::<flecs::Singleton>();
        world.component::<Hub>().add_trait::<flecs::Singleton>();

        let hub = parse(map::HUB).unwrap_or_else(|error| {
            panic!("the hub map does not parse: {error}");
        });
        let arenas: Vec<Loaded> = map::ARENAS
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let spec = parse(source).unwrap_or_else(|error| {
                    panic!("map {index} does not parse: {error}");
                });
                Loaded {
                    spec,
                    origin: region_origin(
                        i32::try_from(index).expect("a server cannot hold 2^31 maps") + 1,
                    ),
                }
            })
            .collect();

        let hub_origin = region_origin(HUB_REGION);
        let hub = Loaded {
            spec: hub,
            origin: hub_origin,
        };

        let mut blocks = Blocks::empty(world);
        world.get::<&AsyncRuntime>(|runtime| {
            for map in core::iter::once(&hub).chain(&arenas) {
                stamp(&mut blocks, runtime, map);
            }
        });

        // After the hub's own brushes, or the floor would be stamped over the
        // podiums. The ring is generated rather than written into `hub.map`
        // because it has to have exactly as many podiums as there are
        // registered kits, and the map file has no way to ask.
        selector::build(world, hub_origin);
        stamp_podiums(&mut blocks, world);
        stand_mobs(world);

        world.set(blocks);

        world.set(Hub {
            spawns: hub.spawns(),
        });
        // The first map is live before anyone connects, so a player who joins
        // an empty server and immediately fills the lobby still lands on a real
        // arena rather than on whatever `Arena::default` happened to say.
        if let Some(first) = arenas.first() {
            world.set(first.arena());
        }
        world.set(MapRotation {
            maps: arenas,
            next: 0,
        });

        // hyperion's join path reads `Position` with a hard `get`, so a player
        // with none never reaches the Login packet and sits on "Joining
        // world..." while the proxy happily reports them connected. Handing out
        // the hub's spawn before the join runs is what makes the game joinable
        // at all, and putting it in the hub rather than on an arena is what
        // stops a joiner from landing in the middle of a running match.
        world
            .observer::<flecs::OnSet, &Hub>()
            .with(id::<Uuid>())
            .without(id::<hyperion::simulation::Position>())
            .each_entity(|entity, hub| {
                entity.set(hyperion::simulation::Position::from(
                    hub.spawn(*entity.id()),
                ));
            });

        // The map for the *next* match is chosen when the last one ends, not
        // when the next one starts. `Lobby::scatter` reads the `Arena`
        // singleton on the transition into `Preparing` and an observer of that
        // same transition would run after it, putting everyone on the previous
        // map. Choosing a phase earlier means the singleton is always already
        // right, and no file under `src/module/` has to know maps rotate.
        world
            .observer_named::<PhaseChanged, ()>("smash::rotate_map")
            // `Lobby`, because a payload event only reaches an observer whose
            // term is the id the emit named, and `lobby.rs` names `Lobby`. A
            // term of `Arena` compiles, registers and never fires, which is
            // what left every match on the first map.
            .with(Lobby::id())
            .each_iter(|it, _, ()| {
                let event = *it.param();
                if event.to != Phase::Waiting {
                    return;
                }
                let world = it.world();
                world.get::<&mut MapRotation>(|rotation| {
                    if rotation.maps.is_empty() {
                        return;
                    }
                    rotation.next = (rotation.next + 1) % rotation.maps.len();
                });
                let arena = world.get::<&MapRotation>(|rotation| {
                    rotation.maps.get(rotation.next).map(Loaded::arena)
                });
                if let Some(arena) = arena {
                    world.set(arena);
                }
            });

        // The plinth colours, recomputed from who is playing what and written
        // only where they disagree with the world.
        //
        // A poll rather than an observer on the relation, because the thing
        // that frees a mob most often is a player disconnecting, and that is an
        // entity being destroyed rather than an edge being removed by anybody
        // this file could watch. Reading the world back and writing only the
        // difference is what keeps this from being a second source of truth:
        // the blocks are the display, and `selector::plinths` is the only place
        // that says what they should say.
        world
            .system_named::<&mut Blocks>("smash::render_podiums")
            .each_iter(|it, _, blocks| {
                let world = it.world();
                for (at, block) in selector::plinths(&world) {
                    let Some(kind) = BlockKind::from_str(block.trim_start_matches("minecraft:"))
                    else {
                        continue;
                    };
                    let wanted = BlockState::from_kind(kind);
                    if blocks.get_block(at) == Some(wanted) {
                        continue;
                    }
                    if let Err(error) = blocks.set_block(at, wanted) {
                        tracing::warn!("could not repaint the podium at {at}: {error:?}");
                    }
                }
            });

        // Back to the hub when the results screen is over. The game half ends a
        // match by resetting lives and health; where players physically go is a
        // hosting question, so it is answered here.
        world
            .observer_named::<PhaseChanged, ()>("smash::return_to_hub")
            .with(Lobby::id())
            .each_iter(|it, _, ()| {
                let event = *it.param();
                if event.to != Phase::Waiting {
                    return;
                }
                let world = it.world();
                let hub = world.cloned::<&Hub>();
                let mut players = Vec::new();
                world
                    .query::<&PlayerId>()
                    .with(Player::id())
                    .build()
                    .each_entity(|player, id| players.push((*player.id(), *id)));
                world.get::<&ServerHandle>(|server| {
                    for (index, player) in players {
                        server.teleport(player, hub.spawn(index));
                        server.set_spectating(player, false);
                    }
                });
            });
    }
}

/// Write the kit selector's wool into the world for the first time.
///
/// Stamped at whatever the current state says, which at boot is every podium
/// free, and then kept in step by `smash::render_podiums` below.
fn stamp_podiums(blocks: &mut Blocks, world: &World) {
    for (at, block) in selector::plinths(world) {
        set_named(blocks, at, block);
    }
}

/// Stand a real mob on each podium.
///
/// The half of the selector the game cannot build for itself: `selector` says
/// which mob belongs where and this turns each name into a hyperion entity.
/// `Channel` is what makes it visible, and visible to players who join later:
/// the proxy tracks the channel's position and asks the server for the spawn
/// packets when somebody comes into range, so a mob spawned at boot with
/// nobody connected is still there when the first player walks up to it.
///
/// `(StandsOn, podium)` is the whole point. A click arrives naming an entity,
/// and this edge is what turns that entity back into the kit it offers.
///
/// # Panics
/// If a kit names a mob that is not a mob. Same reason [`set_named`] panics on
/// a block that is not a block: a podium with nothing on it is a kit nobody
/// can pick, and the hole would be the only report.
fn stand_mobs(world: &World) {
    for (podium, at, name) in selector::mobs(world) {
        let kind = EntityKind::named(name).unwrap_or_else(|| panic!("{name:?} is not a mob"));
        #[expect(
            clippy::cast_precision_loss,
            reason = "hub-local coordinates, tens of blocks"
        )]
        let position = Vec3::new(at.x as f32 + 0.5, at.y as f32, at.z as f32 + 0.5);
        world
            .entity()
            .add_enum(kind)
            .set(MobPosition::new(position.x, position.y, position.z))
            // Facing the middle of the ring, so the row of them looks at a
            // player walking through it rather than all one way.
            .set(Yaw::new(facing(position)))
            .set(Pitch::new(0.0))
            .set(Velocity::new(0.0, 0.0, 0.0))
            .add(id::<EntityChannel>())
            .add((selector::StandsOn, podium))
            .add(id::<Spawn>());
    }
}

/// The yaw that points from `at` back at the hub's centre.
fn facing(at: Vec3) -> f32 {
    (-at.x).atan2(at.z).to_degrees()
}

/// Set the block at `at`, naming the block by its vanilla id.
///
/// # Panics
/// If the id names no block. A mistyped wool colour would otherwise be a podium
/// with a hole in it, and the hole would be the only report.
fn set_named(blocks: &mut Blocks, at: IVec3, block: &str) {
    let kind = BlockKind::from_str(block.trim_start_matches("minecraft:"))
        .unwrap_or_else(|| panic!("{block:?} is not a block"));
    if let Err(error) = blocks.set_block(at, BlockState::from_kind(kind)) {
        tracing::warn!("could not put {block} at {at}: {error:?}");
    }
}

/// Write one map's blocks into the world.
fn stamp(blocks: &mut Blocks, runtime: &AsyncRuntime, map: &Loaded) {
    let (min, max) = bounds(map);

    // `set_block` refuses a chunk that is not in the cache, and the cache is
    // only filled on demand by a player walking near. Forcing every column the
    // map touches to load first is what makes the writes land.
    let (min_chunk, max_chunk): (IVec3, IVec3) = (min >> 4, max >> 4);
    for x in min_chunk.x..=max_chunk.x {
        for z in min_chunk.z..=max_chunk.z {
            blocks.block_and_load(
                I16Vec2::new(
                    i16::try_from(x).expect("map is within 2^19 blocks of the origin"),
                    i16::try_from(z).expect("map is within 2^19 blocks of the origin"),
                ),
                runtime,
            );
        }
    }

    let mut placed = 0usize;
    let mut unknown: Option<&'static str> = None;
    for brush in &map.spec.brushes {
        brush.each_block(|at, block| {
            let Some(kind) = BlockKind::from_str(block.trim_start_matches("minecraft:")) else {
                unknown = Some(block);
                return;
            };
            let position = IVec3::new(at[0], at[1], at[2]) + map.origin;
            if blocks
                .set_block(position, BlockState::from_kind(kind))
                .is_ok()
            {
                placed += 1;
            }
        });
    }

    assert!(
        unknown.is_none(),
        "map {:?} names a block that does not exist: {:?}",
        map.spec.name,
        unknown
    );

    stand_on_something(blocks, map);
    tracing::info!(
        "built {:?} by {:?}: {placed} blocks, kill plane y={}",
        map.spec.name,
        map.spec.author,
        map.spec.kill_y
    );
}

/// Refuse a map whose spawn points hang in the air.
///
/// A spawn one block too high looks harmless and is not: hyperion decides
/// whether a player is on the ground by reading the block at `ceil(y) - 1`, so
/// a player placed a block above the floor is airborne from the moment they
/// arrive, and every ability the kits gate on standing still (Fissure, Seismic
/// Slam) answers "You must be on the ground" and never fires. All four arenas
/// shipped with that off-by-one and nothing noticed, because the only symptom
/// is an ability that silently does nothing.
fn stand_on_something(blocks: &Blocks, map: &Loaded) {
    for spawn in map.spawns() {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a spawn is within a few hundred blocks of its region"
        )]
        let below = IVec3::new(
            spawn.x.floor() as i32,
            spawn.y.ceil() as i32 - 1,
            spawn.z.floor() as i32,
        );
        let solid = blocks.get_block(below).is_some_and(|block| !block.is_air());
        assert!(
            solid,
            "map {:?} has a spawn at {spawn} with nothing under it at {below}; a player put there \
             is airborne and cannot use a grounded ability",
            map.spec.name
        );
    }
}

/// The block-space bounding box of every brush in a map, before the region
/// offset. Spawn points are included so a map whose spawns float off the edge of
/// its own geometry still gets its chunks loaded.
fn bounds(map: &Loaded) -> (IVec3, IVec3) {
    let mut min = IVec3::MAX;
    let mut max = IVec3::MIN;
    let mut widen = |at: IVec3| {
        min = min.min(at);
        max = max.max(at);
    };

    for brush in &map.spec.brushes {
        brush.each_block(|at, _| widen(IVec3::new(at[0], at[1], at[2]) + map.origin));
    }
    for spawn in map.spawns() {
        widen(spawn.as_ivec3());
    }

    if min.x > max.x {
        // A map with no geometry at all. One chunk is enough to fail visibly.
        return (map.origin, map.origin);
    }
    (min, max)
}
