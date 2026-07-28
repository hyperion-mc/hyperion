//! Wire tests for entity spawning, movement and tracked data.
//!
//! # Provenance
//!
//! Every expected value is read from `tests/fixtures/vanilla.json`, which
//! `nix/java/VanillaEncoder.java` writes by driving the real `StreamCodec`s in
//! the pinned `server-26.2.jar`. So a failure here says this crate disagrees
//! with Mojang's encoder, and the `minecraft-encoder-fixtures` flake check
//! says the committed fixtures still match the jar.
//!
//! Two inputs are still spelled out rather than looked up, and both are inputs
//! rather than expectations: the truncated body in
//! [`set_entity_data_rejects_a_body_that_is_not_terminated`], which is a
//! malformed packet no encoder would produce, and the ten serializer ids in
//! [`entity_data_serializer_ids_match_the_server`], which are compared against
//! the harness's own `entity_data_serializer.*` values.

mod vanilla_fixtures;

use hyperion_minecraft_proto::{
    BlockPos, Decode, Encode, Reader, RegistryId, Result, Uuid, VarInt, VarLong, Writer,
    item::{DataComponentPatch, ItemStack, Slot, nbt::NbtScan},
    packets::play::entity::{
        AddEntity, DamageEvent, DataValues, EntityDataSerializer, EquipmentEntry, EquipmentSlot,
        Interact, SetEntityData, SetEntityMotion, SetEquipment, lp_vec3, pack_degrees,
        unpack_degrees,
    },
    text::Component,
    types::{InteractionHand, Vec3},
};

/// The `minecraft:entity_type` id of `minecraft:pig` in 26.2.
fn pig() -> RegistryId {
    RegistryId(vanilla_fixtures::number("entity_type_id.pig"))
}

/// The `minecraft:item` id of `minecraft:diamond_sword`.
fn diamond_sword() -> i32 {
    vanilla_fixtures::number("item_id.diamond_sword")
}

/// The `minecraft:item` id of `minecraft:stone`.
fn stone() -> i32 {
    vanilla_fixtures::number("item_id.stone")
}

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
    for (degrees, fixture) in [
        (0.0_f32, "packed_degrees.0"),
        (90.0, "packed_degrees.90"),
        (-45.5, "packed_degrees.-45.5"),
        (179.9, "packed_degrees.179.9"),
        (-179.9, "packed_degrees.-179.9"),
    ] {
        let expected = i8::try_from(vanilla_fixtures::number(fixture)).expect("a byte");
        assert_eq!(pack_degrees(degrees), expected, "{fixture}");
    }
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
const LP_VECTORS: &[(&str, [f64; 3])] = &[
    ("zero", [0.0, 0.0, 0.0]),
    ("subnormal", [1.0e-6, -1.0e-6, 0.0]),
    ("small", [0.25, -0.5, 0.125]),
    ("scale_one_exact", [1.0, -1.0, 0.5]),
    ("scale_three", [2.0, -1.0, 3.0]),
    ("scale_four", [1.5, -3.25, 2.0]),
    ("scale_large", [100.5, -0.5, 20.0]),
    ("clamped", [1.0e12, -1.0e12, 0.0]),
    ("nan", [f64::NAN, 1.0, f64::NAN]),
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
    for (name, components) in LP_VECTORS {
        let mut writer = Writer::new();
        lp_vec3::encode(&vec3(*components), &mut writer).expect("encode");
        assert_eq!(
            vanilla_fixtures::hex(writer.as_slice()),
            vanilla_fixtures::get(&format!("lp_vec3.{name}")),
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
    for (name, _) in LP_VECTORS {
        let fixture = format!("lp_vec3.{name}");
        let bytes = vanilla_fixtures::bytes(&fixture);
        let mut reader = Reader::new(&bytes);
        let decoded = lp_vec3::decode(&mut reader).expect("decode");
        reader.finish().expect("consumed");

        let mut writer = Writer::new();
        lp_vec3::encode(&decoded, &mut writer).expect("encode");
        assert_eq!(
            vanilla_fixtures::hex(writer.as_slice()),
            vanilla_fixtures::get(&fixture),
            "{fixture}"
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
        &vanilla_fixtures::bytes("packet.set_entity_motion.small"),
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
        vanilla_fixtures::bytes("packet.set_entity_motion.zero")
    );
}

#[test]
fn interact_is_generated_and_round_trips() {
    // The packet this whole mechanism exists for. Its `location` is a
    // `Vec3#LP_STREAM_CODEC`, which cost the packet its entire Rust type until
    // `protocol.json` gained a way to say "one field, hand-written codec"
    // (hyperion-mc/hyperion#1006). If a future change loses that, the type
    // disappears again and this stops compiling rather than going quiet.
    //
    // Serverbound, so there is no vanilla fixture to hold it against; the
    // bytes of the codec itself are covered by `lp_vec3_matches_the_server`
    // above, which does compare against the server.
    let packet = Interact {
        entity_id: 0x2A,
        hand: InteractionHand::OffHand,
        location: vec3([0.25, -0.5, 0.125]),
        using_secondary_action: true,
    };
    let bytes = encode(&packet);
    // entity id, hand, six velocity bytes, flag: nothing has a length prefix,
    // so a `location` that silently became three f64s would show up here.
    assert_eq!(bytes.len(), 1 + 1 + 6 + 1, "{bytes:02x?}");

    let mut reader = Reader::new(&bytes);
    let back = Interact::decode(&mut reader).expect("decode");
    assert_eq!(back.entity_id, packet.entity_id);
    assert_eq!(back.hand, packet.hand);
    assert_eq!(back.using_secondary_action, packet.using_secondary_action);
    // Quantised, so the vector comes back close rather than equal.
    assert!((back.location.x - packet.location.x).abs() < 1e-3);
}

// --- spawning -------------------------------------------------------------

#[test]
fn add_entity_matches_the_server() {
    round_trip_lossy(
        &AddEntity {
            id: 0x2A,
            uuid: PROFILE_ID,
            r#type: pig(),
            x: 1.5,
            y: 64.0625,
            z: -2.25,
            movement: vec3([0.25, -0.5, 0.125]),
            x_rot: pack_degrees(12.5),
            y_rot: pack_degrees(-45.5),
            y_head_rot: pack_degrees(179.9),
            data: 7,
        },
        &vanilla_fixtures::bytes("packet.add_entity"),
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
        .push(
            9,
            EntityDataSerializer::ItemStack,
            &stack(diamond_sword(), 3),
        )
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
    let expected = vanilla_fixtures::bytes("packet.set_entity_data");
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
        &vanilla_fixtures::bytes("packet.set_entity_data.empty"),
    );
}

#[test]
fn set_entity_data_rejects_a_body_that_is_not_terminated() {
    // Hand-made rather than a fixture: this is a body the harness cannot
    // produce, being the first entry of `packet.set_entity_data` with its
    // terminator cut off.
    let bytes = [0x2A, 0x00, 0x00, 0x21];
    let mut reader = Reader::new(&bytes);
    SetEntityData::decode(&mut reader).expect_err("unterminated run");
}

#[test]
fn entity_data_serializer_ids_match_the_server() {
    // The ten the harness pinned. Nothing else in the table is exercised by a
    // fixture, which is why the ids are stated here rather than assumed.
    for (serializer, fixture) in [
        (EntityDataSerializer::Byte, "byte"),
        (EntityDataSerializer::Int, "int"),
        (EntityDataSerializer::Long, "long"),
        (EntityDataSerializer::Float, "float"),
        (EntityDataSerializer::String, "string"),
        (EntityDataSerializer::Component, "component"),
        (
            EntityDataSerializer::OptionalComponent,
            "optional_component",
        ),
        (EntityDataSerializer::ItemStack, "item_stack"),
        (EntityDataSerializer::Boolean, "boolean"),
        (EntityDataSerializer::BlockPos, "block_pos"),
    ] {
        let name = format!("entity_data_serializer.{fixture}");
        assert_eq!(
            serializer.to_raw(),
            vanilla_fixtures::number(&name),
            "{name}"
        );
    }
}

// --- equipment ------------------------------------------------------------

#[test]
fn equipment_slot_ordinals_match_the_server() {
    // `SetEquipment` sends the ordinal, not `EquipmentSlot.STREAM_CODEC`'s id,
    // and the two disagree on exactly these two constants.
    assert_eq!(
        EquipmentSlot::Offhand.id(),
        vanilla_fixtures::number("equipment_slot.offhand")
    );
    assert_eq!(
        EquipmentSlot::Head.id(),
        vanilla_fixtures::number("equipment_slot.head")
    );
    for (ordinal, slot) in EquipmentSlot::ALL.iter().enumerate() {
        let raw = i32::try_from(ordinal).expect("eight slots");
        assert_eq!(slot.id(), raw);
        assert_eq!(EquipmentSlot::from_id(raw), Some(*slot));
    }
    assert_eq!(EquipmentSlot::from_id(8), None);
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
                    slot: EquipmentSlot::Mainhand,
                    item: stack(diamond_sword(), 1),
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Head,
                    item: Slot::Empty,
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Saddle,
                    item: stack(stone(), 64),
                },
            ],
        },
        &vanilla_fixtures::bytes("packet.set_equipment"),
    );
}

#[test]
fn a_single_equipment_slot_carries_no_continuation_bit() {
    equipment_round_trip(
        &SetEquipment {
            entity: 1,
            slots: vec![EquipmentEntry {
                slot: EquipmentSlot::Offhand,
                item: stack(stone(), 2),
            }],
        },
        &vanilla_fixtures::bytes("packet.set_equipment.single"),
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
            source_type: RegistryId(vanilla_fixtures::number("damage_type_id.arrow")),
            source_cause_id: Some(0),
            source_direct_id: None,
            source_position: None,
        },
        &vanilla_fixtures::bytes("packet.damage_event"),
    );
    round_trip(
        &DamageEvent {
            entity_id: 1,
            source_type: RegistryId(vanilla_fixtures::number("damage_type_id.generic")),
            source_cause_id: Some(7),
            source_direct_id: Some(9),
            source_position: Some(vec3([1.5, -2.0, 0.25])),
        },
        &vanilla_fixtures::bytes("packet.damage_event.full"),
    );
}
