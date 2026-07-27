//! What a client needs out of `minecraft:update_tags`.
//!
//! `tests/configuration.rs` proves the packet's *shape* against Mojang's
//! encoder. This file proves the *contents* are enough to join with, which is
//! a different failure: a well-formed empty tag map is exactly what
//! disconnected every real 26.2 client with "Network Protocol Error".

use std::collections::{HashMap, HashSet};

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Writer, nbt::Tag, packets::configuration::UpdateTags, registry_data,
    tag_data,
};

fn encoded_body() -> Vec<u8> {
    let mut writer = Writer::new();
    tag_data::VanillaTags
        .encode(&mut writer)
        .expect("the tag map encodes");
    writer.into_vec()
}

#[test]
fn encoded_length_matches_the_generated_constant() {
    assert_eq!(encoded_body().len(), tag_data::ENCODED_LEN);
}

/// The blob is written by hand rather than by [`UpdateTags`], so the only
/// thing making it a valid packet is that [`UpdateTags`] can read it back.
#[test]
fn the_body_decodes_as_update_tags() {
    let bytes = encoded_body();
    let mut reader = Reader::new(&bytes);
    let decoded = UpdateTags::decode(&mut reader).expect("decode");
    reader.finish().expect("body fully consumed");

    assert_eq!(decoded.tags.len(), tag_data::ALL.len());
    for (read, generated) in decoded.tags.iter().zip(tag_data::ALL) {
        assert_eq!(read.registry, generated.registry);
        assert_eq!(read.tags.len(), generated.len());
    }
}

/// Every tag name in the map, with the registries it appears in.
fn sent_tags() -> HashMap<&'static str, HashSet<&'static str>> {
    let mut sent: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    for set in tag_data::ALL {
        for (name, _) in set.iter() {
            sent.entry(name).or_default().insert(set.registry);
        }
    }
    sent
}

/// Collect every `#namespace:path` string in a tag tree.
///
/// A `HolderSet` written by `RegistryOps` is either an element name, a list, or
/// the tag's own name with a `#` in front, and only the last of those is a
/// reference. The colon is what separates it from the hex colours the biome
/// elements carry, which also begin with `#`.
fn collect_references(tag: &Tag<'_>, into: &mut HashSet<String>) {
    match tag {
        Tag::String(value) => {
            if let Some(name) = value.strip_prefix('#')
                && name.contains(':')
            {
                into.insert(name.to_owned());
            }
        }
        Tag::List(list) => {
            for element in list.as_slice() {
                collect_references(element, into);
            }
        }
        Tag::Compound(compound) => {
            for (_, value) in compound.iter() {
                collect_references(value, into);
            }
        }
        _ => {}
    }
}

/// The invariant a client enforces on itself, checked before it has to.
///
/// A registry element naming a tag the server never sent fails to parse, one
/// failed element fails the whole registry load, and that fails
/// `finish_configuration`. So every tag the shipped registry contents name has
/// to be in the shipped tag map.
///
/// Derived from the contents rather than listed, because a list of the 43
/// names that happened to be missing in 26.2 is precisely the thing that
/// breaks on the next version bump.
#[test]
fn every_tag_the_registries_reference_is_sent() {
    let sent = sent_tags();

    let mut referenced = HashSet::new();
    for registry in registry_data::ALL {
        for (element, payload) in registry.iter() {
            let mut reader = Reader::new(payload);
            let tag = Tag::decode(&mut reader)
                .unwrap_or_else(|error| panic!("{} / {element}: {error}", registry.registry));
            reader
                .finish()
                .unwrap_or_else(|error| panic!("{} / {element}: {error}", registry.registry));
            collect_references(&tag, &mut referenced);
        }
    }

    assert!(
        !referenced.is_empty(),
        "no tag references found at all, which means this test is reading the registry payloads \
         wrong rather than that the data is clean"
    );

    let mut absent: Vec<&String> = referenced
        .iter()
        .filter(|name| !sent.contains_key(name.as_str()))
        .collect();
    absent.sort();
    assert!(
        absent.is_empty(),
        "{} of {} referenced tags are not in the tag map: {absent:?}",
        absent.len(),
        referenced.len()
    );
}

/// The three registries the 26.2 elements actually name tags in. Named because
/// a map that lost one of them would still pass a "not empty" check.
#[test]
fn the_registries_a_client_parses_against_are_present() {
    for registry in ["minecraft:item", "minecraft:block", "minecraft:entity_type"] {
        let set = tag_data::by_name(registry)
            .unwrap_or_else(|| panic!("{registry} has no tags in the map"));
        assert!(!set.is_empty(), "{registry} has an empty tag set");
    }
}

/// A tag holds ids, so a tag naming an id past the end of its registry points
/// at nothing on the client. Only the dynamic registries can be checked here:
/// the static ones are the client's own and this crate carries no element
/// count for them.
#[test]
fn dynamic_registry_tags_index_inside_their_registry() {
    for set in tag_data::ALL {
        let Some(contents) = registry_data::by_name(set.registry) else {
            continue;
        };
        for (name, payload) in set.iter() {
            let mut reader = Reader::new(payload);
            let count = reader.var_int().expect("tag element count");
            for _ in 0..count {
                let id = reader.var_int().expect("tag element id");
                assert!(
                    usize::try_from(id).is_ok_and(|id| id < contents.len()),
                    "{}#{name} names id {id}, but the registry has {} elements",
                    set.registry,
                    contents.len()
                );
            }
            reader.finish().expect("id list fully consumed");
        }
    }
}
