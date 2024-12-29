use std::collections::{HashSet, VecDeque};

use flecs_ecs::core::{Entity, EntityView, EntityViewGet, WorldGet, WorldProvider};
use hyperion::{BlockState, glam::IVec3, simulation::blocks::Blocks};
use hyperion_clap::CommandPermission;
use rayon::iter::ParallelIterator;

use crate::OreVeins;

#[derive(clap::Parser, CommandPermission, Debug)]
#[command(name = "replace")]
#[command_permission(group = "Admin")]
pub struct ReplaceCommand;

/// Picks a random ore based on weighted probabilities
/// Weights are roughly based on real Minecraft ore distribution
fn pick_ore() -> BlockState {
    // Total weight is 100 for easy percentage calculation
    const WEIGHTS: &[(BlockState, u32)] = &[
        (BlockState::COAL_ORE, 16),
        (BlockState::COPPER_ORE, 8),
        (BlockState::IRON_ORE, 4),
        (BlockState::GOLD_ORE, 2),
        (BlockState::EMERALD_ORE, 1),
    ];

    let total_weight: u32 = WEIGHTS.iter().map(|(_, w)| w).sum();
    let mut roll = fastrand::u32(0..total_weight);

    for (block, weight) in WEIGHTS {
        if roll < *weight {
            return *block;
        }
        roll -= weight;
    }

    // Fallback (should never happen due to math)
    BlockState::STONE
}

/// When replacing an existing ore, picks what to replace it with
/// Uses a simple ratio system for clarity
fn pick_given_ore(ore: BlockState) -> BlockState {
    // Ratio of 5:3:2 for stone:cobble:original
    const TOTAL_PARTS: u32 = 10;
    let roll = fastrand::u32(0..TOTAL_PARTS);

    match roll {
        0..=4 => BlockState::STONE,       // 5/10 = 50%
        5..=7 => BlockState::COBBLESTONE, // 3/10 = 30%
        _ => ore,                         // 2/10 = 20%
    }
}

const ADJACENT: [IVec3; 6] = [
    IVec3::new(-1, 0, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(0, -1, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(0, 0, -1),
    IVec3::new(0, 0, 1),
];

/// Groups connected positions in 3D space
/// Returns a vector of groups, where each group is a vector of connected positions
fn group(positions: &HashSet<IVec3>) -> Vec<Vec<IVec3>> {
    let mut visited: HashSet<IVec3> = HashSet::default();
    let mut groups: Vec<Vec<IVec3>> = Vec::new();

    // Iterate through all positions
    for &start_pos in positions {
        // Skip if already visited
        if visited.contains(&start_pos) {
            continue;
        }

        // Start a new group
        let mut current_group = Vec::new();
        let mut queue = VecDeque::new();

        queue.push_back(start_pos);
        visited.insert(start_pos);
        current_group.push(start_pos);

        // Process all connected positions
        while let Some(current_pos) = queue.pop_front() {
            // Check all adjacent positions
            for offset in ADJACENT {
                let neighbor = current_pos + offset;

                // If neighbor exists in positions and hasn't been visited
                if positions.contains(&neighbor) && !visited.contains(&neighbor) {
                    visited.insert(neighbor);
                    queue.push_back(neighbor);
                    current_group.push(neighbor);
                }
            }
        }

        // Add the completed group
        groups.push(current_group);
    }

    groups
}

impl hyperion_clap::MinecraftCommand for ReplaceCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        world.get::<&mut Blocks>(|blocks| {
            let started_time = std::time::Instant::now();

            let concrete_positions: HashSet<_> =
                blocks.par_scan_for(BlockState::PINK_CONCRETE).collect();

            let scan_time = started_time.elapsed();

            let len = concrete_positions.len();

            let groups = group(&concrete_positions);

            world.get::<&mut OreVeins>(|ore_veins| {
                for group in groups {
                    let group_ore = pick_ore();
                    for position in group {
                        let actual_ore = pick_given_ore(group_ore);
                        blocks.set_block(position, actual_ore).unwrap();

                        if actual_ore == BlockState::COBBLESTONE || actual_ore == BlockState::STONE
                        {
                            continue;
                        }
                        ore_veins.insert(position);
                    }
                }
            });

            let elapsed = started_time.elapsed();

            // 317ms debug
            // -> 37ms release
            let msg = hyperion::chat!(
                "Replaced {len} concrete blocks in {elapsed:?} with scan time {scan_time:?}"
            );

            world.get::<&hyperion::net::Compose>(|compose| {
                caller
                    .entity_view(world)
                    .get::<&hyperion::net::ConnectionId>(|stream| {
                        let mut bundle = hyperion::net::DataBundle::new(compose, system);
                        bundle.add_packet(&msg).unwrap();
                        bundle.unicast(*stream).unwrap();
                    });
            });
        });
    }
}
