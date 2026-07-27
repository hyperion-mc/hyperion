//! Wire tests for entity spawning, movement and tracked data.
//!
//! # Provenance
//!
//! Every hex string below was printed by `nix/java/VanillaEncoder.java`'s
//! `playPackets()`, which drives the real `StreamCodec`s in the pinned
//! `server-26.2.jar`. The vectors are not read from
//! `tests/fixtures/vanilla.json` the way `tests/world.rs` reads its own,
//! because that harness does not currently run against 26.2: it names
//! `EntityType.PIG`, which moved to `EntityTypes`, it builds item stacks before
//! anything has bound item component prototypes, and its
//! `PalettedContainerFactory` call wants a registry a bare harness has not
//! loaded. The values here came from running that same file with those three
//! defects patched out, so they are Mojang's bytes; once the harness builds,
//! these belong in `vanilla.json` and this file should read them from
//! [`vanilla_fixtures`] instead. ENG-10435 tracks the three defects.

mod vanilla_fixtures;

use hyperion_minecraft_proto::{
    BlockPos, Decode, Encode, Reader, RegistryId, Result, Uuid, VarInt, VarLong, Writer,
    item::{DataComponentPatch, ItemStack, Slot, nbt::NbtScan},
    packets::play::entity::{
        AddEntity, DamageEvent, DataValues, EntityDataSerializer, EquipmentEntry, EquipmentSlot,
        SetEntityData, SetEntityMotion, SetEquipment, lp_vec3, pack_degrees, unpack_degrees,
    },
    text::Component,
    types::Vec3,
};

/// The `minecraft:entity_type` id of `minecraft:pig` in 26.2, from the harness.
const PIG: RegistryId = RegistryId(100);
/// The `minecraft:item` id of `minecraft:diamond_sword`, from the harness.
const DIAMOND_SWORD: i32 = 964;
/// The `minecraft:item` id of `minecraft:stone`, from the harness.
const STONE: i32 = 1;

/// A `UUID` with every byte distinct, so a transposed half shows up.
const PROFILE_ID: Uuid = Uuid(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);

/// No item in these vectors carries a component, so nothing ever asks for a
/// tag length; a scanner that refuses is therefore also an assertion that
/// nothing tried.
struct NoNbt;

impl NbtScan for NoNbt {
    fn tag_len(&self, _bytes: &[u8]) -> Result<usize> {
        panic!("no fixture in this file carries an NBT-shaped component")
    }
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

fn encode<T: Encode>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        vanilla_fixtures::hex(&encode(value)),
        vanilla_fixtures::hex(bytes),
        "encoding mismatch"
    );
    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

/// A round trip through bytes rather than through the value, for a packet
/// carrying a quantised velocity.
///
/// [`lp_vec3`] loses precision by design, so a decoded velocity never equals
/// the one that was sent. Re-encoding is still exact, so a decoder that shifted
/// a component or read the scale wrong shows up as different bytes.
fn round_trip_lossy<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + std::fmt::Debug,
{
    assert_eq!(
        vanilla_fixtures::hex(&encode(value)),
        vanilla_fixtures::hex(bytes),
        "encoding mismatch"
    );
    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(
        vanilla_fixtures::hex(&encode(&decoded)),
        vanilla_fixtures::hex(bytes),
        "re-encoding mismatch"
    );
}

fn stack(item: i32, count: i32) -> Slot<'static> {
    Slot::Occupied(ItemStack {
        count,
        item,
        components: DataComponentPatch::default(),
    })
}

// --- rotation -------------------------------------------------------------

#[test]
fn packed_degrees_match_mth() {
    // `Mth.packDegrees` floors, so 179.9 and -179.9 land on opposite ends of
    // the byte rather than on the same value; a round-to-nearest port would
    // agree on 0 and 90 and differ on both of these.
    assert_eq!(pack_degrees(0.0), 0);
    assert_eq!(pack_degrees(90.0), 64);
    assert_eq!(pack_degrees(-45.5), -33);
    assert_eq!(pack_degrees(179.9), 127);
    assert_eq!(pack_degrees(-179.9), -128);
}

#[test]
fn unpacking_a_degree_byte_inverts_the_packing() {
    for packed in i8::MIN..=i8::MAX {
        assert_eq!(pack_degrees(unpack_degrees(packed)), packed);
    }
}

// --- velocity -------------------------------------------------------------

/// Every `Vec3.LP_STREAM_CODEC` case the harness printed: the zero shortcut,
/// scales inside the two marker bits, the first scale needing a continuation,
/// and the clamping of a vector past `ABS_MAX_VALUE`.
const LP_VECTORS: &[(&str, [f64; 3], &str)] = &[
    ("zero", [0.0, 0.0, 0.0], "00"),
    ("subnormal", [1.0e-6, -1.0e-6, 0.0], "00"),
    ("small", [0.25, -0.5, 0.125], "f97f8ffe8002"),
    ("scale_one_exact", [1.0, -1.0, 0.5], "f1ffbffe0003"),
    ("scale_three", [2.0, -1.0, 3.0], "4b55fffcaaab"),
    ("scale_four", [1.5, -3.25, 2.0], "fcbfbffe300201"),
    ("scale_large", [100.5, -0.5, 20.0], "6dfd9956febb19"),
    ("clamped", [1.0e12, -1.0e12, 0.0], "f7ff7ffe0003ffffffff0f"),
    ("nan", [f64::NAN, 1.0, f64::NAN], "f9ff7ffffff9"),
];

const fn vec3(components: [f64; 3]) -> Vec3 {
    Vec3 {
        x: components[0],
        y: components[1],
        z: components[2],
    }
}

#[test]
fn lp_vec3_matches_the_server() {
    for (name, components, expected) in LP_VECTORS {
        let mut writer = Writer::new();
        lp_vec3::encode(&vec3(*components), &mut writer).expect("encode");
        assert_eq!(
            vanilla_fixtures::hex(writer.as_slice()),
            *expected,
            "lp_vec3.{name}"
        );
    }
}

#[test]
fn lp_vec3_decodes_to_a_value_that_re_encodes_identically() {
    // The encoding is lossy, so the round trip that can be asserted is the one
    // through the bytes rather than the one through the value: a decoder that
    // shifted a component would produce a different vector and, on the way
    // back, different bytes.
    for (name, _, expected) in LP_VECTORS {
        let bytes = hex(expected);
        let mut reader = Reader::new(&bytes);
        let decoded = lp_vec3::decode(&mut reader).expect("decode");
        reader.finish().expect("consumed");

        let mut writer = Writer::new();
        lp_vec3::encode(&decoded, &mut writer).expect("encode");
        assert_eq!(
            vanilla_fixtures::hex(writer.as_slice()),
            *expected,
            "lp_vec3.{name}"
        );
    }
}

#[test]
fn set_entity_motion_matches_the_server() {
    round_trip_lossy(
        &SetEntityMotion {
            id: 0x2A,
            movement: vec3([0.25, -0.5, 0.125]),
        },
        &hex("2af97f8ffe8002"),
    );
}

#[test]
fn a_still_entity_costs_one_velocity_byte() {
    // The zero shortcut is what keeps a crowd of standing players off the
    // wire; a codec that always wrote six bytes would look correct here.
    assert_eq!(
        encode(&SetEntityMotion {
            id: 0x2A,
            movement: vec3([0.0, 0.0, 0.0]),
        }),
        hex("2a00")
    );
}

// --- spawning -------------------------------------------------------------

#[test]
fn add_entity_matches_the_server() {
    round_trip_lossy(
        &AddEntity {
            id: 0x2A,
            uuid: PROFILE_ID,
            r#type: PIG,
            x: 1.5,
            y: 64.0625,
            z: -2.25,
            movement: vec3([0.25, -0.5, 0.125]),
            x_rot: pack_degrees(12.5),
            y_rot: pack_degrees(-45.5),
            y_head_rot: pack_degrees(179.9),
            data: 7,
        },
        &hex(
            "2a00112233445566778899aabbccddeeff643ff80000000000004050040000000000c0020000000000\
             00f97f8ffe800208df7f07",
        ),
    );
}

// --- tracked data ---------------------------------------------------------

/// The twelve values the harness put in `packet.set_entity_data`, in order.
///
/// Ten different serializers, including both an occupied and an empty item
/// stack and both a present and an absent optional component, so an entry that
/// mis-sizes a value shifts everything after it.
fn harness_data_values() -> DataValues {
    let mut values = DataValues::new();
    values
        .push(0, EntityDataSerializer::Byte, &0x21_u8)
        .expect("byte");
    values
        .push(1, EntityDataSerializer::Int, &VarInt(-1234))
        .expect("int");
    values
        .push(2, EntityDataSerializer::Long, &VarLong(1_234_567_890_123))
        .expect("long");
    values
        .push(3, EntityDataSerializer::Float, &12.5_f32)
        .expect("float");
    values
        .push(4, EntityDataSerializer::String, &"hello")
        .expect("string");
    values
        .push(5, EntityDataSerializer::Boolean, &true)
        .expect("boolean");
    values
        .push(6, EntityDataSerializer::Component, &text("hi"))
        .expect("component");
    values
        .push(
            7,
            EntityDataSerializer::OptionalComponent,
            &Some(text("hi")),
        )
        .expect("optional component");
    values
        .push(
            8,
            EntityDataSerializer::OptionalComponent,
            &Option::<Component<'_>>::None,
        )
        .expect("absent optional component");
    values
        .push(9, EntityDataSerializer::ItemStack, &stack(DIAMOND_SWORD, 3))
        .expect("item stack");
    values
        .push(10, EntityDataSerializer::ItemStack, &Slot::Empty)
        .expect("empty item stack");
    values
        .push(11, EntityDataSerializer::BlockPos, &BlockPos::new(1, -2, 3))
        .expect("block pos");
    values
}

fn text(literal: &str) -> Component<'_> {
    Component::text(literal)
}

#[test]
fn set_entity_data_matches_the_server() {
    let values = harness_data_values();
    let expected = hex(
        "2a0000210101aef6ffff0f0202cb89ec8ff72303034148000004040568656c6c6f0508010605080002686907\
         06010800026869080600090703c40700000a07000b0a0000004000003ffeff",
    );
    round_trip(
        &SetEntityData {
            id: 0x2A,
            packed_items: values.as_bytes(),
        },
        &expected,
    );
    assert_eq!(
        expected.last(),
        Some(&0xFF),
        "the run is terminated rather than counted"
    );
}

#[test]
fn set_entity_data_with_nothing_to_send_is_the_terminator_alone() {
    // A codec that wrote a zero count instead would produce the same two bytes
    // for a different reason, so this also pins the id it is paired with.
    round_trip(
        &SetEntityData {
            id: 1,
            packed_items: &[],
        },
        &hex("01ff"),
    );
}

#[test]
fn set_entity_data_rejects_a_body_that_is_not_terminated() {
    let bytes = hex("2a000021");
    let mut reader = Reader::new(&bytes);
    SetEntityData::decode(&mut reader).expect_err("unterminated run");
}

#[test]
fn entity_data_serializer_ids_match_the_server() {
    // The ten the harness pinned. Nothing else in the table is exercised by a
    // fixture, which is why the ids are stated here rather than assumed.
    assert_eq!(EntityDataSerializer::Byte.to_raw(), 0);
    assert_eq!(EntityDataSerializer::Int.to_raw(), 1);
    assert_eq!(EntityDataSerializer::Long.to_raw(), 2);
    assert_eq!(EntityDataSerializer::Float.to_raw(), 3);
    assert_eq!(EntityDataSerializer::String.to_raw(), 4);
    assert_eq!(EntityDataSerializer::Component.to_raw(), 5);
    assert_eq!(EntityDataSerializer::OptionalComponent.to_raw(), 6);
    assert_eq!(EntityDataSerializer::ItemStack.to_raw(), 7);
    assert_eq!(EntityDataSerializer::Boolean.to_raw(), 8);
    assert_eq!(EntityDataSerializer::BlockPos.to_raw(), 10);
}

// --- equipment ------------------------------------------------------------

#[test]
fn equipment_slot_ordinals_match_the_server() {
    // `SetEquipment` sends the ordinal, not `EquipmentSlot.STREAM_CODEC`'s id,
    // and the two disagree on exactly these two constants.
    assert_eq!(EquipmentSlot::OffHand.to_raw(), 1);
    assert_eq!(EquipmentSlot::Head.to_raw(), 5);
    for (ordinal, slot) in EquipmentSlot::VALUES.iter().enumerate() {
        let raw = u8::try_from(ordinal).expect("eight slots");
        assert_eq!(slot.to_raw(), raw);
        assert_eq!(EquipmentSlot::from_raw(raw), Some(*slot));
    }
    assert_eq!(EquipmentSlot::from_raw(8), None);
}

fn equipment_round_trip(value: &SetEquipment<'_>, bytes: &[u8]) {
    assert_eq!(
        vanilla_fixtures::hex(&encode(value)),
        vanilla_fixtures::hex(bytes),
        "encoding mismatch"
    );
    let mut reader = Reader::new(bytes);
    let decoded = SetEquipment::decode(&mut reader, &NoNbt).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

#[test]
fn set_equipment_matches_the_server() {
    // Three entries, so the continuation bit is set twice and clear once, with
    // an empty stack in the middle because that is the entry the item codec
    // shortens to a single byte and therefore the one a length-confused reader
    // would slide off.
    equipment_round_trip(
        &SetEquipment {
            entity: 0x2A,
            slots: vec![
                EquipmentEntry {
                    slot: EquipmentSlot::MainHand,
                    item: stack(DIAMOND_SWORD, 1),
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Head,
                    item: Slot::Empty,
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Saddle,
                    item: stack(STONE, 64),
                },
            ],
        },
        &hex("2a8001c407000085000740010000"),
    );
}

#[test]
fn a_single_equipment_slot_carries_no_continuation_bit() {
    equipment_round_trip(
        &SetEquipment {
            entity: 1,
            slots: vec![EquipmentEntry {
                slot: EquipmentSlot::OffHand,
                item: stack(STONE, 2),
            }],
        },
        &hex("010102010000"),
    );
}

#[test]
fn set_equipment_refuses_to_write_an_empty_slot_list() {
    // The client reads the first entry before testing the continuation bit, so
    // an empty packet would make it consume the next packet's id as a slot.
    let mut writer = Writer::new();
    SetEquipment {
        entity: 1,
        slots: Vec::new(),
    }
    .encode(&mut writer)
    .expect_err("an entry-less packet is unreadable, not empty");
}

// --- damage ---------------------------------------------------------------

#[test]
fn damage_event_folds_absence_into_the_entity_id() {
    // `writeOptionalEntityId` sends `id + 1`, so a present id of zero and an
    // absent one differ by exactly the byte this asserts.
    round_trip(
        &DamageEvent {
            entity_id: 0x2A,
            source_type: RegistryId(3),
            source_cause_id: Some(0),
            source_direct_id: None,
            source_position: None,
        },
        &hex("2a03010000"),
    );
    round_trip(
        &DamageEvent {
            entity_id: 1,
            source_type: RegistryId(0),
            source_cause_id: Some(7),
            source_direct_id: Some(9),
            source_position: Some(vec3([1.5, -2.0, 0.25])),
        },
        &hex("0100080a013ff8000000000000c0000000000000003fd0000000000000"),
    );
}
