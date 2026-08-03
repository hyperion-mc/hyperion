//! Turning the ids a 1.20.1 world is stored as into the ids 26.2 sends.
//!
//! hyperion reads its world out of anvil region files written by Minecraft
//! 1.20.1, and valence hands those back as `BlockState`s numbered against
//! 1.20.1's registry of 24135 states. Protocol 776 numbers 32366 states, and
//! the two numberings agree nowhere past `minecraft:air`. Sending a 1.20.1 id
//! to a 26.2 client is not an error at any layer: the client looks the number
//! up, finds some block, and renders a world made of the wrong thing.
//!
//! So the translation goes through names. Every 1.20.1 state is a block name
//! plus a value for each of the block's properties, and
//! [`hyperion_minecraft_proto::block_state::state_id`] turns exactly that back
//! into a 776 id. That is a string lookup per state, which is far too slow to
//! do per block -- a 24-section chunk is 98304 of them -- so it runs once for
//! all 24135 states and the answers live in [`BLOCK_STATES`].
//!
//! Biomes have the same problem in miniature and are handled the same way, in
//! [`biome_ids`].
//!
//! Collision shapes go the same way for the same reason, in
//! [`collision_shapes`]: valence's table describes 1.20.1's geometry, and what
//! an entity stops against has to be the geometry the client is rendering.

use std::sync::LazyLock;

use hyperion_minecraft_proto::{
    block_state,
    collision_shape::{self, CollisionBox},
};
use valence_generated::block::BlockState;
use valence_protocol::Ident;
use valence_registry::{BiomeRegistry, RegistryIdx, biome::BiomeId};

use crate::net::protocol::registries;

/// Blocks 1.20.1 and 26.2 both have under different names.
///
/// A rename is the only way a block can go missing between the two versions,
/// because blocks are added and renamed but not removed. Each entry names the
/// version that renamed it, so that a future bump can tell a stale workaround
/// from a live one.
const RENAMED_BLOCKS: &[(&str, &str)] = &[
    // 1.20.3 renamed `grass` to `short_grass`, pairing it with `tall_grass`.
    ("grass", "short_grass"),
    // 1.21.5 added the copper chains and put the metal in the iron one's name.
    ("chain", "iron_chain"),
];

/// Protocol 776 state id for every 1.20.1 block state, indexed by raw id.
///
/// Built on first use rather than generated, because the input is valence's
/// table and the output is the proto crate's, and neither is ours to extend
/// with a cross-reference.
pub static BLOCK_STATES: LazyLock<Box<[u32]>> = LazyLock::new(build_block_states);

/// The 776 state id for a 1.20.1 block state.
#[must_use]
pub fn block_state(state: BlockState) -> u32 {
    // Every raw id below `max_raw` has an entry, and `to_raw` cannot produce
    // anything else, so the index is in range by construction.
    BLOCK_STATES[usize::from(state.to_raw())]
}

/// The 26.2 collision boxes of a 1.20.1 block state, in the block's own
/// coordinates.
///
/// Use this rather than [`BlockState::collision_shapes`], which answers out of
/// valence's checked-in 1.20.1 table. They agree on all but one of the 24135
/// states a 1.20.1 world can hold, which the `shapes_changed_since_1_20_1`
/// test below both measures and pins; the reason to ask 26.2 anyway is that
/// 26.2 is what the client deciding where a player may stand is running.
///
/// A state with no boxes is passed through -- air, a torch, tall grass -- so
/// an empty slice is the answer for "nothing to collide with" and not a
/// failure to look it up.
#[must_use]
pub fn collision_shapes(state: BlockState) -> &'static [CollisionBox] {
    // Total by construction: every id [`block_state`] returns comes out of the
    // same jar's registry that the shape table was extracted from, and the two
    // tables assert against each other's state count at compile time.
    collision_shape::collision_shape(block_state(state))
        .expect("every protocol 776 state id has a collision shape")
}

/// The name 26.2 knows a 1.20.1 block by.
fn renamed(name: &str) -> &str {
    RENAMED_BLOCKS
        .iter()
        .find_map(|(old, new)| (*old == name).then_some(*new))
        .unwrap_or(name)
}

fn build_block_states() -> Box<[u32]> {
    let count = usize::from(BlockState::max_raw()) + 1;
    let mut table = vec![block_state::AIR; count];
    let mut unmapped: Vec<String> = Vec::new();
    let mut properties: Vec<(&'static str, &'static str)> = Vec::new();
    let mut qualified = String::with_capacity(64);

    for raw in 0..=BlockState::max_raw() {
        let Some(state) = BlockState::from_raw(raw) else {
            // valence's ids are dense, so this cannot happen; if it ever does,
            // the gap is air rather than a panic because a gap is not a block
            // anyone stored.
            continue;
        };

        let kind = state.to_kind();
        qualified.clear();
        qualified.push_str("minecraft:");
        qualified.push_str(renamed(kind.to_str()));

        properties.clear();
        for name in kind.props() {
            let value = state
                .get(*name)
                .expect("a block kind's own property has a value in every one of its states");
            properties.push((name.to_str(), value.to_str()));
        }

        match block_state::state_id(&qualified, &properties) {
            Some(id) => table[usize::from(raw)] = id,
            None => unmapped.push(format!("{qualified}{properties:?}")),
        }
    }

    // A state that silently became air would be a hole in the world that only
    // shows up as a player falling through it, so this refuses to start
    // instead. The list is truncated because an unmapped *block* is 1 to 100
    // unmapped states and the first few name it just as well.
    assert!(
        unmapped.is_empty(),
        "{} of {count} 1.20.1 block states have no protocol 776 equivalent, starting with {:?}. \
         Either RENAMED_BLOCKS is missing an entry or block_state.rs was generated from the wrong \
         version.",
        unmapped.len(),
        &unmapped[..unmapped.len().min(8)]
    );

    table.into_boxed_slice()
}

/// Protocol 776 biome ids for a 1.20.1 biome registry, indexed by [`BiomeId`].
///
/// The 1.20.1 registry hyperion builds from its own codec and the one 26.2
/// synchronises are both sorted by name, but they are different lengths, so
/// the ids line up for a prefix and then drift. Mapping through names is the
/// only thing that stays right when a version adds a biome.
///
/// A biome 26.2 does not have becomes `minecraft:plains`, unlike an unmapped
/// block: a wrong biome is a wrong grass colour, and the world is still there.
#[must_use]
pub fn biome_ids(biomes: &BiomeRegistry) -> Vec<BiomeId> {
    let fallback = registries::WORLDGEN_BIOME
        .id_of("minecraft:plains")
        .expect("the 26.2 biome registry has minecraft:plains");

    let mut ids = vec![BiomeId::from_index(0); biomes.iter().count()];
    for (id, name, _) in biomes.iter() {
        let translated = registries::WORLDGEN_BIOME
            .id_of(name.as_str())
            .unwrap_or_else(|| {
                tracing::warn!("no 26.2 biome named {name}; sending minecraft:plains instead");
                fallback
            });
        ids[id.to_index()] = BiomeId::from_index(usize::try_from(translated).unwrap_or(0));
    }
    ids
}

/// The name-to-id map [`super::loader::parse::parse_chunk`] resolves an anvil
/// biome palette with, already carrying 776 ids.
///
/// Nothing outside this module reads a [`BiomeId`] as an index into the 1.20.1
/// registry, so translating here rather than at encode time keeps the
/// conversion off the chunk encoder entirely.
#[must_use]
pub fn biome_name_to_id(biomes: &BiomeRegistry) -> std::collections::BTreeMap<Ident, BiomeId> {
    let translated = biome_ids(biomes);
    biomes
        .iter()
        .map(|(id, name, _)| (name, translated[id.to_index()]))
        .collect()
}

#[cfg(test)]
mod tests {
    use valence_generated::block::{BlockState, PropName, PropValue};

    use super::{block_state, collision_shapes};

    /// Named 26.2 geometry, reached through the same call the collision path
    /// makes.
    ///
    /// Written against the extracted table rather than from memory: each of
    /// these was read out of `collision-shapes.json` first and is here to say
    /// that the translation lands on the right row, not to restate the game.
    #[test]
    fn known_blocks_have_their_2_6_2_shapes() {
        const CUBE: [f32; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];

        assert_eq!(collision_shapes(BlockState::STONE), &[CUBE]);
        assert_eq!(collision_shapes(BlockState::SOUL_SAND), &[[
            0.0, 0.0, 0.0, 1.0, 0.875, 1.0
        ]]);

        // The three slab states, which are the reason a shape is asked for per
        // state and not per block.
        let slab = |value| BlockState::OAK_SLAB.set(PropName::Type, value);
        assert_eq!(collision_shapes(slab(PropValue::Bottom)), &[[
            0.0, 0.0, 0.0, 1.0, 0.5, 1.0
        ]]);
        assert_eq!(collision_shapes(slab(PropValue::Top)), &[[
            0.0, 0.5, 0.0, 1.0, 1.0, 1.0
        ]]);
        assert_eq!(collision_shapes(slab(PropValue::Double)), &[CUBE]);

        // A fence post stands 1.5 blocks high, above the cell it belongs to,
        // so a consumer clamping a shape to the unit cube would let a player
        // walk over one.
        let fence = BlockState::OAK_FENCE
            .set(PropName::North, PropValue::False)
            .set(PropName::South, PropValue::False)
            .set(PropName::East, PropValue::False)
            .set(PropName::West, PropValue::False)
            .set(PropName::Waterlogged, PropValue::False);
        assert_eq!(collision_shapes(fence), &[[
            0.375, 0.0, 0.375, 0.625, 1.5, 0.625
        ]]);

        // Nothing to collide with is an empty slice, not a missing row.
        assert!(collision_shapes(BlockState::AIR).is_empty());
        assert!(collision_shapes(BlockState::TORCH).is_empty());
        assert!(collision_shapes(BlockState::WATER).is_empty());
    }

    /// Every 1.20.1 state resolves to a 26.2 shape.
    ///
    /// [`collision_shapes`] expects rather than returns an option, on the
    /// grounds that the id it looks up came out of the same registry the table
    /// did. This is what makes that reasoning checked: a state whose
    /// translation landed outside the table would panic here rather than in a
    /// tick.
    #[test]
    fn every_1_20_1_state_has_a_2_6_2_shape() {
        for raw in 0..=BlockState::max_raw() {
            let Some(state) = BlockState::from_raw(raw) else {
                continue;
            };
            let _: &[[f32; 6]] = collision_shapes(state);
        }
    }

    /// What swapping the table actually changed, measured rather than assumed.
    ///
    /// The honest result is: almost nothing. Of the 24135 states a 1.20.1
    /// world can hold, exactly one has different geometry in 26.2 --
    /// `pitcher_crop[age=0, half=upper]`, which valence gives the bulb box and
    /// 26.2 gives no collision at all -- and that state cannot be placed in a
    /// world, because a pitcher crop only grows its upper half at age 3. So
    /// this change fixes no collision anybody could have hit. What it fixes is
    /// the source: the shapes now come from the version the client is running,
    /// and `nix flake check` fails if the committed table drifts from the jar.
    ///
    /// Kept as a ratchet. A jar bump that moves another block's shape fails
    /// here and names it, which is the difference between knowing what a
    /// version bump changed and hoping.
    #[test]
    fn shapes_changed_since_1_20_1() {
        let expected = BlockState::PITCHER_CROP
            .set(PropName::Age, PropValue::_0)
            .set(PropName::Half, PropValue::Upper);

        let mut changed = Vec::new();
        for raw in 0..=BlockState::max_raw() {
            let Some(state) = BlockState::from_raw(raw) else {
                continue;
            };
            let old: Vec<[f32; 6]> = state
                .collision_shapes()
                .map(|shape| {
                    let (min, max) = (shape.min().as_vec3(), shape.max().as_vec3());
                    [min.x, min.y, min.z, max.x, max.y, max.z]
                })
                .collect();
            if old != collision_shapes(state) {
                changed.push(state);
            }
        }

        assert_eq!(
            changed,
            vec![expected],
            "the shapes 26.2 disagrees with 1.20.1 about have moved; each entry is a block whose \
             collision geometry a player will feel change"
        );
    }

    /// Named states, checked against `block_state.rs`'s own table.
    ///
    /// `minecraft:chest` is here because its properties enumerate in a
    /// different order than they are declared: the ids run
    /// `[facing, type, waterlogged]` while the block declares
    /// `[type, facing, waterlogged]`. A translation that indexed by
    /// declaration order would land on a chest facing the wrong way, which is
    /// exactly the kind of wrong that renders fine.
    #[test]
    fn known_blocks_translate_by_name() {
        let expect = |name: &str, props: &[(&str, &str)]| {
            hyperion_minecraft_proto::block_state::state_id(name, props).unwrap()
        };

        assert_eq!(block_state(BlockState::AIR), 0);
        assert_eq!(
            block_state(BlockState::STONE),
            expect("minecraft:stone", &[])
        );
        assert_eq!(
            block_state(BlockState::GRASS_BLOCK),
            expect("minecraft:grass_block", &[("snowy", "false")])
        );

        // The two blocks 26.2 renamed.
        assert_eq!(
            block_state(BlockState::GRASS),
            expect("minecraft:short_grass", &[])
        );
        assert_eq!(
            block_state(BlockState::CHAIN),
            expect("minecraft:iron_chain", &[
                ("axis", "y"),
                ("waterlogged", "false")
            ])
        );

        // Properties out of declaration order.
        let chest = BlockState::CHEST
            .set(PropName::Facing, PropValue::West)
            .set(PropName::Type, PropValue::Left)
            .set(PropName::Waterlogged, PropValue::False);
        assert_eq!(
            block_state(chest),
            expect("minecraft:chest", &[
                ("facing", "west"),
                ("type", "left"),
                ("waterlogged", "false")
            ])
        );
    }

    /// The guard in `build_block_states` only fires at startup, so this is
    /// what makes a missing rename a test failure rather than a crash in
    /// production.
    #[test]
    fn every_1_20_1_state_has_a_776_equivalent() {
        // Air is only the right answer for air; anything else mapping to it
        // would mean the table was filled in and never overwritten.
        for raw in 1..=BlockState::max_raw() {
            let state = BlockState::from_raw(raw).unwrap();
            assert_ne!(
                block_state(state),
                0,
                "{:?} translated to air",
                state.to_kind().to_str()
            );
        }
    }
}
