//! `entity_type.rs` and `generated/registry.rs` describe the same registry.
//!
//! Both are generated from `protocol.json`, but by different scripts and into
//! different shapes, so nothing about the build forces them to agree. A drift
//! between them would show up as an entity spawning as the type next to the one
//! that was asked for, which is exactly the failure the table exists to stop.

use hyperion_minecraft_proto::{
    RegistryId,
    entity_type::{ENTITY_TYPE_COUNT, ENTITY_TYPES, EntityType, entity_type},
    generated::registry::ENTITY_TYPE,
};

#[test]
fn table_matches_the_registry() {
    assert_eq!(ENTITY_TYPES.len(), ENTITY_TYPE.entries.len());
    assert_eq!(ENTITY_TYPES.len(), ENTITY_TYPE_COUNT);

    for (index, entry) in ENTITY_TYPES.iter().enumerate() {
        assert_eq!(entry.name(), ENTITY_TYPE.entries[index]);
        // The position in the table is the id, which is what lets a decoder
        // index straight into it.
        assert_eq!(entry.id(), RegistryId(i32::try_from(index).unwrap()));
    }
}

#[test]
fn names_resolve_to_their_own_entries() {
    for entry in ENTITY_TYPES {
        assert_eq!(entity_type(entry.name()), Some(*entry));
    }
    assert_eq!(
        entity_type("minecraft:boat"),
        None,
        "split per wood in 1.21.2"
    );
    assert_eq!(
        entity_type("pig"),
        None,
        "the namespace is part of the name"
    );
}

/// The two the 1.20.1 numbering got wrong, pinned so a bad regeneration that
/// happened to stay self-consistent still fails.
#[test]
fn ids_are_the_26_2_ones() {
    assert_eq!(EntityType::PIG.id(), RegistryId(100));
    assert_eq!(EntityType::PLAYER.id(), RegistryId(156));
}
