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

use std::sync::LazyLock;

use hyperion_minecraft_proto::block_state;
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

    use super::block_state;

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
