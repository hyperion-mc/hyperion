//! Wire tests for the combat and player status packets.
//!
//! # Provenance
//!
//! Unlike `tests/play_entity.rs`, these vectors were not printed by
//! `nix/java/VanillaEncoder.java`: its `playPackets()` does not build any of
//! these packets, and adding them means regenerating
//! `tests/fixtures/vanilla.json`. Every layout here is fixed-width or a
//! `VarInt`, with no branch anywhere in it, so the bytes are derived from the
//! field order in each packet's `STREAM_CODEC` and spelled out in the comment
//! above the vector. ENG-10448 tracks moving them onto the harness.
//!
//! What they defend is the field encoding rather than the field order: three
//! of these packets mix a `VarInt` id with a fixed-width one, and
//! `ClientboundEntityEventPacket` is the outlier that writes its entity id as
//! a plain big-endian `int`. A generator that reached for `VarInt` everywhere
//! would still round trip, and would still be wrong.

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, RegistryId, Writer,
    packets::play::{
        clientbound::{
            PlayerCombatKill, SetExperience, SetHealth, UpdateAttributes,
            update_attributes::{AttributeSnapshot, AttributeSnapshotModifier},
        },
        entity::{EntityEvent, HurtAnimation, RemoveEntities},
    },
    text::Component,
};

fn render(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _unused = write!(out, "{byte:02x}");
        out
    })
}

fn hex(text: &str) -> Vec<u8> {
    assert!(
        text.len().is_multiple_of(2),
        "hex fixture has an odd length"
    );
    (0..text.len() / 2)
        .map(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex fixture digit")
        })
        .collect()
}

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    assert_eq!(
        render(writer.as_slice()),
        render(bytes),
        "encoding mismatch"
    );

    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

#[test]
fn hurt_animation_pairs_a_var_int_id_with_a_float_yaw() {
    // `id` 42 as a VarInt, then 90.0 as a big-endian f32.
    round_trip(&HurtAnimation { id: 42, yaw: 90.0 }, &hex("2a42b40000"));
}

#[test]
fn entity_event_writes_a_fixed_width_entity_id() {
    // `ByteBufCodecs.INT`, not `VAR_INT`: 42 costs four bytes here and one in
    // every neighbouring packet. Then the event id as a single byte, 3 being
    // `Entity.DEATH`.
    round_trip(
        &EntityEvent {
            entity_id: 42,
            event_id: 3,
        },
        &hex("0000002a03"),
    );
}

#[test]
fn player_combat_kill_carries_the_death_screen_message() {
    // `playerId` 42 as a VarInt, then the component as network NBT. A literal
    // with no style collapses to a bare string tag, so this is
    // TAG_String, length 2, "hi".
    let message = Component::text("hi");
    round_trip(
        &PlayerCombatKill {
            player_id: 42,
            message: message.to_tag(),
        },
        &hex("2a0800026869"),
    );
}

#[test]
fn remove_entities_is_a_counted_list_of_var_ints() {
    // Count 2, then 1 and 300; 300 is the two-byte VarInt that catches a
    // list written as fixed-width ints.
    round_trip(&RemoveEntities(vec![1, 300]), &hex("0201ac02"));

    // Nothing to remove is a legal packet, and an encoder that skipped the
    // count would produce an empty body that decodes as garbage.
    round_trip(&RemoveEntities(Vec::new()), &hex("00"));
}

#[test]
fn set_health_puts_the_food_level_between_two_floats() {
    // 20.0 health, food 20 as a VarInt, 5.0 saturation.
    round_trip(
        &SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        },
        &hex("41a000001440a00000"),
    );
}

#[test]
fn set_experience_leads_with_the_bar_fill() {
    // The progress float comes first, then the level and the running total as
    // VarInts. Transposing the two ints is invisible at level zero, so the
    // level here is not the total.
    round_trip(
        &SetExperience {
            experience_progress: 0.5,
            experience_level: 30,
            total_experience: 0,
        },
        &hex("3f0000001e00"),
    );
}

#[test]
fn update_attributes_nests_a_counted_list_of_modifiers() {
    // Entity 42, one snapshot: attribute 1 (`minecraft:armor`), base 3.0 as an
    // f64, and no modifiers, which is still a count.
    round_trip(
        &UpdateAttributes {
            entity_id: 42,
            values: vec![AttributeSnapshot {
                attribute: RegistryId(1),
                base: 3.0,
                modifiers: Vec::new(),
            }],
        },
        &hex("2a0101400800000000000000"),
    );

    // With a modifier: id "hi" as a length-prefixed string, amount 0.5 as an
    // f64, operation 1 as a VarInt.
    round_trip(
        &UpdateAttributes {
            entity_id: 42,
            values: vec![AttributeSnapshot {
                attribute: RegistryId(1),
                base: 3.0,
                modifiers: vec![AttributeSnapshotModifier {
                    id: "hi",
                    amount: 0.5,
                    operation: 1,
                }],
            }],
        },
        &hex("2a01014008000000000000010268693fe000000000000001"),
    );
}
