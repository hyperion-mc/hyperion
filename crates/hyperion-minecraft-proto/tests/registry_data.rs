//! The registry contents a client cannot start without.
//!
//! The bytes came out of `RegistrySynchronization.packRegistry` in the pinned
//! server jar, so what is worth checking here is not that they encode -- they
//! did, in Java -- but that this crate's view of them is coherent: that the
//! table lengths agree with the blob, that each element is valid network NBT,
//! and that the fields a client reads exist and hold the values the datapack
//! JSON shows.

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Writer,
    nbt::Tag,
    packets::configuration::RegistryData,
    registry_data::{self, RegistryContents},
};

fn decode_element(registry: &RegistryContents, name: &str) -> Tag<'static> {
    let payload = registry
        .get(name)
        .unwrap_or_else(|| panic!("{} has no {name}", registry.registry));
    let mut reader = Reader::new(payload);
    let tag = Tag::decode(&mut reader).expect("element is valid network NBT");
    reader
        .finish()
        .expect("element NBT consumes its whole payload");
    tag
}

fn compound_field<'a>(tag: &'a Tag<'_>, field: &str) -> &'a Tag<'a> {
    let Tag::Compound(entries) = tag else {
        panic!("expected a compound");
    };
    entries
        .iter()
        .find_map(|(name, value)| (name == field).then_some(value))
        .unwrap_or_else(|| panic!("no field named {field}"))
}

/// Every element's declared length has to match what the blob actually holds,
/// or `iter` walks off the end of one element into the next and every element
/// after the first bad one is silently wrong.
#[test]
fn the_lengths_account_for_every_byte() {
    for registry in registry_data::ALL {
        assert_eq!(
            registry.ids.len(),
            registry.lengths.len(),
            "{} has mismatched tables",
            registry.registry
        );
        let declared: usize = registry.lengths.iter().map(|len| *len as usize).sum();
        assert_eq!(
            declared,
            registry.payloads.len(),
            "{} declares {declared} bytes over {} of payload",
            registry.registry,
            registry.payloads.len()
        );
        assert_eq!(registry.iter().count(), registry.len());
    }
}

#[test]
fn every_element_is_valid_network_nbt() {
    for registry in registry_data::ALL {
        for (id, payload) in registry.iter() {
            let mut reader = Reader::new(payload);
            let tag = Tag::decode(&mut reader)
                .unwrap_or_else(|error| panic!("{} / {id}: {error}", registry.registry));
            reader
                .finish()
                .unwrap_or_else(|error| panic!("{} / {id}: {error}", registry.registry));
            assert!(
                matches!(tag, Tag::Compound(_)),
                "{} / {id} is not a compound",
                registry.registry
            );
        }
    }
}

/// The three a client indexes into: `login` names a dimension type, chunk
/// sections carry biome ids and every chat message carries a chat type id.
#[test]
fn the_registries_a_client_needs_are_present() {
    assert_eq!(registry_data::DIMENSION_TYPE.len(), 4);
    assert!(registry_data::DIMENSION_TYPE.get("minecraft:overworld").is_some());
    assert!(registry_data::DIMENSION_TYPE.get("minecraft:the_nether").is_some());
    assert!(registry_data::DIMENSION_TYPE.get("minecraft:the_end").is_some());

    assert_eq!(registry_data::WORLDGEN_BIOME.len(), 66);
    assert!(registry_data::WORLDGEN_BIOME.get("minecraft:plains").is_some());

    assert_eq!(registry_data::CHAT_TYPE.len(), 7);
    assert!(registry_data::CHAT_TYPE.get("minecraft:chat").is_some());

    for name in [
        "minecraft:dimension_type",
        "minecraft:worldgen/biome",
        "minecraft:chat_type",
    ] {
        assert_eq!(
            registry_data::by_name(name).expect(name).registry,
            name
        );
    }
}

/// Spot-checks against `data/minecraft/dimension_type/overworld.json` from the
/// vanilla data generator. These four fields decide how tall a chunk column is
/// and therefore how many sections `level_chunk_with_light` carries, so a
/// wrong one shows up as a client that disconnects on the first chunk.
#[test]
fn the_overworld_carries_the_height_a_chunk_is_built_against() {
    let overworld = decode_element(&registry_data::DIMENSION_TYPE, "minecraft:overworld");

    assert_eq!(compound_field(&overworld, "min_y"), &Tag::Int(-64));
    assert_eq!(compound_field(&overworld, "height"), &Tag::Int(384));
    assert_eq!(compound_field(&overworld, "logical_height"), &Tag::Int(384));
    // NBT has no boolean, so `has_skylight: true` is a byte of 1.
    assert_eq!(compound_field(&overworld, "has_skylight"), &Tag::Byte(1));

    // 384 blocks over 16-block sections is 24 chunk sections, and one light
    // section below and above makes 26 entries in each light mask.
    assert_eq!(384 / 16, 24);
}

#[test]
fn a_chat_type_carries_the_decoration_a_client_renders_with() {
    let chat = decode_element(&registry_data::CHAT_TYPE, "minecraft:chat");
    let chat_decoration = compound_field(&chat, "chat");
    assert_eq!(
        compound_field(chat_decoration, "translation_key"),
        &Tag::String("chat.type.text".into())
    );
}

/// The generated table is only useful if it produces a packet body the
/// crate's own `registry_data` decoder reads back.
#[test]
fn the_encoded_packet_round_trips() {
    for registry in registry_data::ALL {
        let mut writer = Writer::new();
        registry.encode(&mut writer).expect("encode");
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = RegistryData::decode(&mut reader).expect("decode");
        reader.finish().expect("fully consumed");

        assert_eq!(decoded.registry, registry.registry);
        assert_eq!(decoded.entries.len(), registry.len());
        for (entry, (id, _)) in decoded.entries.iter().zip(registry.iter()) {
            assert_eq!(entry.id, id);
            assert!(
                entry.data.is_some(),
                "{} / {id} was written without contents",
                registry.registry
            );
        }
    }
}

/// Encoding copies the stored bytes rather than re-serialising a parsed tree,
/// so a decode-then-encode of one element has to be byte-identical or the
/// stored bytes and the NBT codec disagree about something.
#[test]
fn re_encoding_an_element_reproduces_it() {
    for registry in registry_data::ALL {
        for (id, payload) in registry.iter() {
            let mut reader = Reader::new(payload);
            let tag = Tag::decode(&mut reader).expect("decode");
            let mut writer = Writer::new();
            tag.encode(&mut writer).expect("encode");
            assert_eq!(
                writer.as_slice(),
                payload,
                "{} / {id} did not round-trip",
                registry.registry
            );
        }
    }
}
