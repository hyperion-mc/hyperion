use std::{cell::RefCell, rc::Rc};

use flecs_ecs::{
    core::{QueryBuilderImpl, SystemAPI, TermBuilderImpl, World, flecs},
    macros::{Component, observer},
    prelude::Module,
};
use hyperion::{
    runtime::AsyncRuntime,
    simulation::{Position, Uuid, blocks::Blocks},
    valence_protocol::{
        BlockKind,
        math::{IVec2, IVec3, Vec3},
    },
};
use rustc_hash::FxHashMap;

#[derive(Component)]
pub struct SpawnModule;

const RADIUS: i32 = 0;
const SPAWN_MIN_Y: i16 = 3;
const SPAWN_MAX_Y: i16 = 100;

fn position_in_radius() -> IVec2 {
    let x = fastrand::i32(-RADIUS..=RADIUS);
    let z = fastrand::i32(-RADIUS..=RADIUS);

    IVec2::new(x, z)
}

fn random_chunk_in_radius() -> I16Vec2 {
    let pos: IVec2 = position_in_radius() >> 4;
    pos.as_i16vec2()
}

use hyperion::{glam::I16Vec2, valence_protocol::BlockState};
use roaring::RoaringBitmap;
use tracing::info;

pub fn avoid_blocks() -> RoaringBitmap {
    let mut blocks = RoaringBitmap::new();
    let spawnable = [BlockKind::Lava];

    for block in spawnable {
        blocks.insert(u32::from(block.to_raw()));
    }
    blocks
}

impl Module for SpawnModule {
    fn module(world: &World) {
        let positions = Rc::new(RefCell::new(FxHashMap::default()));
        let avoid_blocks = avoid_blocks();

        observer!(
            world,
            flecs::OnSet,
            &Uuid,
            &mut Blocks($),
            &AsyncRuntime($) ,
        )
        .without::<Position>()
        .each_entity({
            let positions = Rc::clone(&positions);
            move |entity, (uuid, blocks, runtime)| {
                let mut positions = positions.borrow_mut();
                let position = *positions
                    .entry(uuid.0)
                    .or_insert_with(|| find_spawn_position(blocks, runtime, &avoid_blocks));

                entity.set(Position::from(position));
            }
        });

        world
            .observer::<flecs::OnRemove, (&Uuid, &Position)>()
            .each(move |(uuid, position)| {
                let mut positions = positions.borrow_mut();
                positions.insert(uuid.0, **position);
            });
    }
}

fn find_spawn_position(
    blocks: &mut Blocks,
    runtime: &AsyncRuntime,
    avoid_blocks: &RoaringBitmap,
) -> Vec3 {
    const MAX_TRIES: usize = 3;
    const FALLBACK_POSITION: Vec3 = Vec3::new(0.0, 120.0, 0.0);

    for _ in 0..MAX_TRIES {
        let chunk = random_chunk_in_radius();
        if let Some(pos) = try_chunk_for_spawn(chunk, blocks, runtime, avoid_blocks) {
            return pos;
        }
    }

    FALLBACK_POSITION
}

fn try_chunk_for_spawn(
    chunk: I16Vec2,
    blocks: &mut Blocks,
    runtime: &AsyncRuntime,
    avoid_blocks: &RoaringBitmap,
) -> Option<Vec3> {
    blocks.block_and_load(chunk, runtime);
    let column = blocks.get_loaded_chunk(chunk)?;

    let candidate_positions: Vec<_> = column
        .blocks_in_range(SPAWN_MIN_Y, SPAWN_MAX_Y)
        .filter(|&(pos, state)| is_valid_spawn_block(pos, state, blocks, avoid_blocks))
        .collect();

    let (position, state) = *fastrand::choice(&candidate_positions)?;
    info!("spawned at {position:?} with state {state:?}");

    let position = IVec3::new(0, 1, 0) + position;
    let position = position.as_vec3() + Vec3::new(0.5, 0.0, 0.5);
    Some(position)
}

pub fn is_valid_spawn_block(
    pos: IVec3,
    state: BlockState,
    blocks: &Blocks,
    avoid_blocks: &RoaringBitmap,
) -> bool {
    const DISPLACEMENTS: [IVec3; 2] = [IVec3::new(0, 1, 0), IVec3::new(0, 2, 0)];

    let Some(ground) = blocks.get_block(pos) else {
        return false;
    };

    if ground.collision_shapes().is_empty() {
        return false;
    }

    if avoid_blocks.contains(u32::from(state.to_raw())) {
        return false;
    }

    for displacement in DISPLACEMENTS {
        let above = pos + displacement;
        if let Some(block) = blocks.get_block(above) {
            if !block.collision_shapes().is_empty() {
                return false;
            }

            if block.is_liquid() {
                return false;
            }
        }
    }

    true
}
