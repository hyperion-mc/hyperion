//! Holds this server's simulation against the real one, tick by tick.
//!
//! No Java, no network and no server run here: the expected numbers are
//! committed traces recorded from the pinned vanilla jar by
//! `nix run .#record-differential-traces`, and `nix flake check` re-records
//! them so a version bump cannot leave them stale. `docs/differential-testing.md`
//! is the whole story, including what these traces do and do not prove.
//!
//! Adding a case is adding a scenario file and its trace. There is deliberately
//! no per-scenario code here.

#![expect(
    clippy::print_stdout,
    reason = "a failing comparison is only useful if it prints the tick and the numbers"
)]

use std::{collections::BTreeSet, fs, path::Path};

use flecs_ecs::{
    core::{EntityViewGet, World, WorldGet},
    macros::Component,
    prelude::Module,
};
use hyperion::{
    BlockKind, BlockState,
    glam::{I16Vec2, IVec3, Vec3},
    runtime::AsyncRuntime,
    simulation::{
        Owner, Pitch, Position, Velocity, Yaw,
        blocks::Blocks,
        entity_kind::EntityKind,
        metadata::{Metadata, arrow::InGround},
        projectile_motion::{SIMULATED, ShakeTime, look_angles},
    },
    spatial::SpatialModule,
};
use serde::Deserialize;
use serial_test::serial;

#[derive(Component)]
struct TestModule;

impl Module for TestModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world.import::<SpatialModule>();
    }
}

/// The committed scenario, as `docs/differential-testing.md` describes it.
///
/// `serde(deny_unknown_fields)` on purpose: a scenario with a misspelled key
/// would otherwise silently fall back to a default and be compared against the
/// wrong recording.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    #[expect(dead_code, reason = "documentation for a reader of the scenario file")]
    description: String,
    ticks: usize,
    #[expect(dead_code, reason = "consumed by the recorder, not by this test")]
    seed: i64,
    entities: Vec<EntitySpec>,
    /// Terrain the scenario wants, and nothing else.
    ///
    /// Absent from every scenario that flies through open sky, which is what
    /// keeps their traces byte-identical: the recorder places nothing and this
    /// replay stamps nothing, so the world both sides run in is the one they
    /// always ran in. A scenario that names blocks opts *itself* into terrain;
    /// there is no global flat-world switch to get wrong.
    #[serde(default)]
    blocks: Vec<BlockSpec>,
    compare: Tolerance,
}

/// One block the scenario puts in the world before anything is fired.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockSpec {
    position: [i32; 3],
    /// A block name, and only a name: `minecraft:stone`, not
    /// `minecraft:stone_slab[type=top]`.
    ///
    /// Both sides place the block's *default* state, which means the two
    /// registries' defaults have to agree. That is not taken on trust -- it is
    /// what the comparison itself checks. A slab that came out `top` on one
    /// side and `bottom` on the other moves the arrow's resting height half a
    /// block, four orders of magnitude outside any tolerance here.
    state: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitySpec {
    id: String,
    #[serde(rename = "type")]
    entity_type: String,
    position: [f64; 3],
    #[serde(default)]
    #[expect(dead_code, reason = "the recorder resolves the impulse; see below")]
    motion: Option<[f64; 3]>,
    #[serde(default)]
    #[expect(dead_code, reason = "the recorder resolves the impulse; see below")]
    launch: Option<serde_json::Value>,
    #[serde(default)]
    #[expect(dead_code, reason = "the recorder resolves the impulse; see below")]
    knockback: Option<serde_json::Value>,
}

/// How far apart the two simulations may be before the case fails.
///
/// Justified rather than tuned; see `docs/differential-testing.md`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tolerance {
    position: f64,
    velocity: f64,
    /// In degrees. Loose next to position and velocity because hyperion aims
    /// with `f32::atan2` where vanilla uses `Mth.atan2`, a table approximation;
    /// the gap is under a thousandth of a degree across every committed
    /// scenario, and the tolerance is orders of magnitude tighter than the
    /// wrong-sign (up to 180) or frozen-rotation (tens of degrees) failures it
    /// exists to catch.
    rotation: f64,
}

#[derive(Deserialize)]
struct Trace {
    scenario: String,
    #[serde(rename = "minecraftVersion")]
    #[expect(
        dead_code,
        reason = "recorded so a reader can see which jar produced this"
    )]
    minecraft_version: String,
    #[expect(
        dead_code,
        reason = "recorded so a reader can see the run was not the default seed"
    )]
    seed: i64,
    ticks: usize,
    /// The wire index of `AbstractArrow.IN_GROUND`, read out of the jar by the
    /// recorder rather than transcribed.
    ///
    /// This is the one number in `metadata::arrow` that nothing else can check.
    /// A field index never appears on the wire, so no packet capture recovers
    /// it, and getting it wrong does not fail to compile or to send -- it sends
    /// a boolean to whichever field Mojang moved into slot 10, and the arrow
    /// quietly does something else on the client. Recording it here costs the
    /// recorder one reflective read and turns a hand-transcribed constant into
    /// one the jar has to agree with. See ENG-12106 for the general case.
    #[serde(rename = "inGroundFieldIndex")]
    in_ground_field_index: u8,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
struct Sample {
    tick: usize,
    entities: std::collections::HashMap<String, State>,
}

#[derive(Deserialize)]
struct State {
    position: [f64; 3],
    velocity: [f64; 3],
    /// The client-facing orientation, `[yaw, pitch]` in degrees, in vanilla's
    /// projectile-entity convention. This is the "wrong heading" the parity
    /// work is about: an arrow whose arc is right but which renders pointing
    /// the wrong way sends a yaw of the wrong sign, or one frozen at its launch
    /// value rather than tracking its velocity.
    rotation: [f64; 2],
    #[expect(
        dead_code,
        reason = "recorded so a scenario can one day assert a despawn"
    )]
    removed: bool,
    /// `AbstractArrow.isInGround`, present only for the kinds that have it.
    ///
    /// The reason a terrain scenario can assert anything at all. A resting
    /// position on its own cannot tell "stopped by the wall" from "still
    /// flying and happening to be there this tick"; this can.
    #[serde(default, rename = "inGround")]
    in_ground: Option<bool>,
    /// `AbstractArrow.shakeTime`, which counts down from seven and so pins the
    /// tick the arrow landed on rather than merely that it did.
    #[serde(default, rename = "shakeTime")]
    shake_time: Option<u8>,
}

/// Narrows a recorded double to the `f32` this server stores.
///
/// The truncation is the subject of the test rather than an accident: vanilla
/// keeps entity state in doubles and hyperion keeps it in `f32` for cache
/// locality, which is the whole reason the tolerances are not zero. See the
/// tolerance section of `docs/differential-testing.md`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "narrowing to f32 is what hyperion does with these numbers"
)]
const fn narrow(value: [f64; 3]) -> [f32; 3] {
    [value[0] as f32, value[1] as f32, value[2] as f32]
}

/// The shorter arc between two angles in degrees, so a reading of 179 against
/// -179 is two degrees apart rather than 358. A rotation is periodic where a
/// position is not, so a plain subtraction would report a full turn of
/// disagreement at the -180/180 seam and fail a scenario that had not moved.
fn angle_delta(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/differential"))
}

/// Maps a protocol entity type name onto the kind this server simulates it as.
///
/// Driven by [`SIMULATED`] rather than by a table here, so a projectile added
/// to the physics module is usable from a scenario the same day.
fn kind_for(entity_type: &str) -> EntityKind {
    SIMULATED
        .iter()
        .map(|(kind, _)| *kind)
        .find(|kind| {
            kind.entity_type()
                .is_some_and(|ty| ty.name() == entity_type)
        })
        .unwrap_or_else(|| {
            panic!(
                "{entity_type} is not in hyperion::simulation::projectile_motion::SIMULATED, so \
                 this server has no physics to compare"
            )
        })
}

/// Puts a scenario's declared blocks into the replay world.
///
/// `HyperionCore` installs `Blocks::empty`, so the replay world starts with no
/// chunks at all -- and `set_block` on an unloaded chunk returns
/// `ChunkNotLoaded` rather than placing anything, which would leave a scenario
/// whose wall silently was not there and a comparison that then blamed the
/// physics. Each chunk is loaded first, through the same `block_and_load` the
/// server uses.
///
/// Nothing happens for a scenario with no blocks, which is every scenario that
/// flies through open sky: their replay world is untouched by this and their
/// committed traces are unchanged.
fn stamp_terrain(world: &World, blocks: &[BlockSpec]) {
    set_terrain(world, blocks, |spec| block_state(&spec.state));
}

/// Takes it back out again.
///
/// Every scenario shares one world -- `HyperionCore` can only be imported once
/// per process -- so a wall left standing would be in the next scenario's sky.
/// Vanilla records each scenario in a fresh level, and this is what makes the
/// replay side match that. The scenarios are also written not to overlap, but
/// relying on that would make every future scenario's author responsible for
/// every past one's geometry.
fn clear_terrain(world: &World, blocks: &[BlockSpec]) {
    set_terrain(world, blocks, |_| BlockState::AIR);
}

fn set_terrain(world: &World, blocks: &[BlockSpec], state_for: impl Fn(&BlockSpec) -> BlockState) {
    if blocks.is_empty() {
        return;
    }

    // Deduplicated and ordered, so a scenario naming twenty blocks in one
    // chunk loads it once and the loads happen in a fixed order. `I16Vec2` is
    // not `Ord`, so the pair is the key and the vector is rebuilt from it.
    let chunks: BTreeSet<(i16, i16)> = blocks
        .iter()
        .map(|spec| {
            (
                i16::try_from(spec.position[0] >> 4).expect("block x is inside the world limit"),
                i16::try_from(spec.position[2] >> 4).expect("block z is inside the world limit"),
            )
        })
        .collect();

    let runtime = world.get::<&AsyncRuntime>(AsyncRuntime::clone);
    world.get::<&mut Blocks>(|store| {
        for (x, z) in chunks {
            store.block_and_load(I16Vec2::new(x, z), &runtime);
        }
        for spec in blocks {
            let state = state_for(spec);
            let position = IVec3::new(spec.position[0], spec.position[1], spec.position[2]);
            store.set_block(position, state).unwrap_or_else(|error| {
                panic!("could not place {} at {position}: {error:?}", spec.state)
            });
        }
    });
}

/// The default state of a block named the way a scenario names it.
///
/// A name and nothing else, so `minecraft:stone`, not
/// `minecraft:stone_slab[type=top]`. See [`BlockSpec::state`] for why the two
/// registries' defaults agreeing is checked by the comparison rather than
/// asserted here.
fn block_state(name: &str) -> BlockState {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    let kind = BlockKind::from_str(bare)
        .unwrap_or_else(|| panic!("no such block in this server's tables: {name}"));
    BlockState::from_kind(kind)
}

/// Replays one scenario and reports the first tick that disagrees.
///
/// Returns the failure as a string rather than asserting, so the caller can
/// run every scenario and report all of them rather than only the first.
/// The worst disagreement seen while replaying a scenario, so a passing run
/// still says how much room is left before the tolerance bites.
struct Headroom {
    position: f64,
    velocity: f64,
    rotation: f64,
}

fn replay(world: &World, scenario: &Scenario, trace: &Trace) -> Result<Headroom, String> {
    assert_eq!(
        scenario.name, trace.scenario,
        "trace is for another scenario"
    );
    assert_eq!(
        scenario.ticks, trace.ticks,
        "trace has a different tick count"
    );
    assert_eq!(
        trace.samples.len(),
        scenario.ticks + 1,
        "a trace carries the state before the first tick plus one sample per tick"
    );

    assert_eq!(
        trace.in_ground_field_index,
        InGround::INDEX,
        "the pinned jar puts AbstractArrow's IN_GROUND at index {}, and \
         hyperion::simulation::metadata::arrow says {}; every arrow this server sends is writing \
         the wrong tracked field",
        trace.in_ground_field_index,
        InGround::INDEX,
    );

    // Terrain first, before anything is spawned to fly into it. The recorder
    // does the same, in `VanillaTrace.placeBlocks`.
    stamp_terrain(world, &scenario.blocks);

    // The owner exists only so the ray cast can exclude the shooter, which is
    // what `update_projectile_positions` does with it.
    let owner = world.entity();

    let initial = &trace.samples[0];
    let entities: Vec<_> = scenario
        .entities
        .iter()
        .map(|spec| {
            let state = initial
                .entities
                .get(&spec.id)
                .unwrap_or_else(|| panic!("trace has no entity called {}", spec.id));

            // The starting velocity is vanilla's own answer, read out of the
            // trace, not recomputed here. `Projectile.shoot` normalises a
            // direction built from Mojang's sine table and this server has no
            // equivalent on this code path, so recomputing it would be
            // comparing an invention against a recording. What is under test is
            // the flight, from whatever state vanilla started it in.
            let [px, py, pz] = narrow(state.position);
            let [vx, vy, vz] = narrow(state.velocity);

            // The launch rotation is seeded from the velocity the same way
            // `Projectile.shoot` seeds vanilla's, through `look_angles` -- the
            // very function the server aims with in flight. Unlike the velocity
            // this is not read out of the trace: rotation is `f(velocity)` on
            // both sides, so computing hyperion's and holding it against
            // vanilla's recorded answer is the parity check the "wrong heading"
            // needs, not an invention compared against a recording.
            let (yaw, pitch) = look_angles(Vec3::new(vx, vy, vz));
            let entity = world.entity();
            entity
                .add_enum(kind_for(&spec.entity_type))
                .set(Position::new(px, py, pz))
                .set(Velocity::new(vx, vy, vz))
                .set(Yaw::new(yaw))
                .set(Pitch::new(pitch))
                .set(Owner::new(*owner));
            (spec.id.clone(), spec.position, entity)
        })
        .collect();

    // Exact equality on purpose: both sides are decimal literals out of two
    // committed JSON files, so any difference at all means the trace was
    // recorded from a different scenario than the one being replayed.
    #[expect(
        clippy::float_cmp,
        reason = "comparing two committed files, not two computed results"
    )]
    for (id, declared, _) in &entities {
        let state = &initial.entities[id];
        assert_eq!(
            *declared, state.position,
            "{id}: the scenario and its trace disagree about where the entity starts"
        );
    }

    let mut outcome = Ok(Headroom {
        position: 0.0,
        velocity: 0.0,
        rotation: 0.0,
    });
    // Every sample, tick 0 included: the seed at tick 0 is where a wrong-sign
    // heading shows up, and each later tick is where a frozen one does. Tick 0
    // is compared without progressing, since it is the state the entity was
    // just built in.
    for (index, sample) in trace.samples.iter().enumerate() {
        if index > 0 {
            world.progress();
        }

        for (id, _, entity) in &entities {
            let expected = &sample.entities[id];
            let (position, velocity, rotation) = entity
                .get::<(&Position, &Velocity, &Yaw, &Pitch)>(|(p, v, yaw, pitch)| {
                    (
                        [f64::from(p.x), f64::from(p.y), f64::from(p.z)],
                        [f64::from(v.0.x), f64::from(v.0.y), f64::from(v.0.z)],
                        [f64::from(**yaw), f64::from(**pitch)],
                    )
                });

            for (axis, name) in ["x", "y", "z"].iter().enumerate() {
                let delta = (position[axis] - expected.position[axis]).abs();
                if let Ok(worst) = &mut outcome {
                    worst.position = worst.position.max(delta);
                }
                if delta > scenario.compare.position {
                    outcome = Err(format!(
                        "{}: {id} position.{name} diverges at tick {}\n  vanilla:  {}\n  \
                         hyperion: {}\n  delta:    {delta:e} (tolerance {:e})",
                        scenario.name,
                        sample.tick,
                        expected.position[axis],
                        position[axis],
                        scenario.compare.position
                    ));
                }

                let delta = (velocity[axis] - expected.velocity[axis]).abs();
                if let Ok(worst) = &mut outcome {
                    worst.velocity = worst.velocity.max(delta);
                }
                if delta > scenario.compare.velocity {
                    outcome = Err(format!(
                        "{}: {id} velocity.{name} diverges at tick {}\n  vanilla:  {}\n  \
                         hyperion: {}\n  delta:    {delta:e} (tolerance {:e})",
                        scenario.name,
                        sample.tick,
                        expected.velocity[axis],
                        velocity[axis],
                        scenario.compare.velocity
                    ));
                }
            }

            // The impact state, compared exactly. No tolerance, and none is
            // possible: these are a flag and a countdown, so "close" has no
            // meaning. An arrow that lands one tick early agrees on position to
            // well inside the tolerance and disagrees here, which is the whole
            // reason they are recorded.
            //
            // Only where vanilla recorded them. A trace of a snowball carries
            // neither, because `ThrowableProjectile` has no such state.
            if let Some(expected_in_ground) = expected.in_ground {
                let in_ground = entity.try_get::<&InGround>(|flag| **flag);
                if in_ground != Some(expected_in_ground) {
                    outcome = Err(format!(
                        "{}: {id} inGround diverges at tick {}\n  vanilla:  \
                         {expected_in_ground}\n  hyperion: {in_ground:?}",
                        scenario.name, sample.tick,
                    ));
                }
            }
            if let Some(expected_shake) = expected.shake_time {
                let shake = entity.try_get::<&ShakeTime>(|shake| shake.0);
                if shake != Some(expected_shake) {
                    outcome = Err(format!(
                        "{}: {id} shakeTime diverges at tick {}\n  vanilla:  {expected_shake}\n  \
                         hyperion: {shake:?}",
                        scenario.name, sample.tick,
                    ));
                }
            }

            // The heading, in the same shape. Yaw and pitch rather than three
            // axes, and the shorter arc between the two angles so a reading of
            // 179 against -179 is two degrees apart, not 358.
            for (axis, name) in ["yaw", "pitch"].iter().enumerate() {
                let delta = angle_delta(rotation[axis], expected.rotation[axis]);
                if let Ok(worst) = &mut outcome {
                    worst.rotation = worst.rotation.max(delta);
                }
                if delta > scenario.compare.rotation {
                    outcome = Err(format!(
                        "{}: {id} {name} diverges at tick {}\n  vanilla:  {}\n  hyperion: {}\n  \
                         delta:    {delta:e} (tolerance {:e})",
                        scenario.name,
                        sample.tick,
                        expected.rotation[axis],
                        rotation[axis],
                        scenario.compare.rotation
                    ));
                }
            }
        }

        // The first divergence is the informative one; every later tick is
        // downstream of it, so the replay stops rather than printing the same
        // drift sixty times.
        if outcome.is_err() {
            break;
        }
    }

    for (_, _, entity) in &entities {
        entity.destruct();
    }
    owner.destruct();
    clear_terrain(world, &scenario.blocks);

    outcome
}

/// Every committed scenario, replayed against its recording.
///
/// One test and one world for all of them, because `HyperionCore` initialises
/// rayon's global thread pool and a second import panics. Each scenario's
/// entities are destroyed on the way out so the next one starts from an empty
/// world.
#[test]
#[serial]
fn scenarios_match_vanilla() {
    let scenario_dir = root().join("scenarios");
    let mut names: Vec<_> = fs::read_dir(&scenario_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", scenario_dir.display()))
        .map(|entry| entry.expect("cannot read scenario directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    names.sort();

    assert!(
        !names.is_empty(),
        "no scenarios in {}",
        scenario_dir.display()
    );

    let world = World::new();
    world.import::<TestModule>();

    let mut failures = Vec::new();
    for path in &names {
        let scenario: Scenario = serde_json::from_str(
            &fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())),
        )
        .unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));

        let trace_path = root()
            .join("traces")
            .join(format!("{}.json", scenario.name));
        let trace: Trace =
            serde_json::from_str(&fs::read_to_string(&trace_path).unwrap_or_else(|e| {
                panic!(
                    "cannot read {}: {e}\nrecord it with: nix run .#record-differential-traces",
                    trace_path.display()
                )
            }))
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", trace_path.display()));

        match replay(&world, &scenario, &trace) {
            Ok(worst) => println!(
                "ok: {} ({} ticks); worst position delta {:e} of {:e}, velocity {:e} of {:e}, \
                 rotation {:e} of {:e}",
                scenario.name,
                scenario.ticks,
                worst.position,
                scenario.compare.position,
                worst.velocity,
                scenario.compare.velocity,
                worst.rotation,
                scenario.compare.rotation
            ),
            Err(failure) => failures.push(failure),
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}
