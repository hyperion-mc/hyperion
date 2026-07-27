//! Wire tests for the combat and player status packets.
//!
//! # Provenance
//!
//! Every expected value but one is read from `tests/fixtures/vanilla.json`,
//! which `nix/java/VanillaEncoder.java` writes by driving the real
//! `StreamCodec`s in the pinned `server-26.2.jar`.
//!
//! The exception is [`entity_event_writes_a_fixed_width_entity_id`].
//! `ClientboundEntityEventPacket`'s only public constructor takes a live
//! `Entity`, which needs a `Level`, which needs a running server, so the
//! harness cannot build one and that vector stays derived from the field
//! order in its `STREAM_CODEC`.
//!
//! What these defend is the field encoding rather than the field order: three
//! of these packets mix a `VarInt` id with a fixed-width one, and
//! `ClientboundEntityEventPacket` is the outlier that writes its entity id as
//! a plain big-endian `int`. A generator that reached for `VarInt` everywhere
//! would still round trip, and would still be wrong.

mod vanilla_fixtures;

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
    round_trip(
        &HurtAnimation { id: 42, yaw: 90.0 },
        &vanilla_fixtures::bytes("packet.hurt_animation"),
    );
}

#[test]
fn entity_event_writes_a_fixed_width_entity_id() {
    // The one vector in this file the harness cannot produce; see the module
    // docs. `ByteBufCodecs.INT`, not `VAR_INT`: 42 costs four bytes here and
    // one in every neighbouring packet. Then the event id as a single byte,
    // 3 being `Entity.DEATH`.
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
        &vanilla_fixtures::bytes("packet.player_combat_kill"),
    );
}

#[test]
fn remove_entities_is_a_counted_list_of_var_ints() {
    // Count 2, then 1 and 300; 300 is the two-byte VarInt that catches a
    // list written as fixed-width ints.
    round_trip(
        &RemoveEntities(vec![1, 300]),
        &vanilla_fixtures::bytes("packet.remove_entities"),
    );

    // Nothing to remove is a legal packet, and an encoder that skipped the
    // count would produce an empty body that decodes as garbage.
    round_trip(
        &RemoveEntities(Vec::new()),
        &vanilla_fixtures::bytes("packet.remove_entities.empty"),
    );
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
        &vanilla_fixtures::bytes("packet.set_health"),
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
        &vanilla_fixtures::bytes("packet.set_experience"),
    );
}

#[test]
fn update_attributes_nests_a_counted_list_of_modifiers() {
    // Entity 42, one snapshot: `minecraft:armor`, base 3.0 as an f64, and no
    // modifiers, which is still a count.
    let armor = RegistryId(vanilla_fixtures::number("attribute_id.armor"));
    round_trip(
        &UpdateAttributes {
            entity_id: 42,
            values: vec![AttributeSnapshot {
                attribute: armor,
                base: 3.0,
                modifiers: Vec::new(),
            }],
        },
        &vanilla_fixtures::bytes("packet.update_attributes"),
    );

    // With a modifier: the id as a length-prefixed string, amount 0.5 as an
    // f64, operation 1 as a VarInt. The id is namespaced because
    // `AttributeModifier.id` is an `Identifier` rather than a string, so the
    // server writes `minecraft:hi` where a bare `hi` went in. This file used
    // to assert the bare form, which nothing on the wire would ever carry.
    round_trip(
        &UpdateAttributes {
            entity_id: 42,
            values: vec![AttributeSnapshot {
                attribute: armor,
                base: 3.0,
                modifiers: vec![AttributeSnapshotModifier {
                    id: "minecraft:hi",
                    amount: 0.5,
                    operation: 1,
                }],
            }],
        },
        &vanilla_fixtures::bytes("packet.update_attributes.modifier"),
    );
}
