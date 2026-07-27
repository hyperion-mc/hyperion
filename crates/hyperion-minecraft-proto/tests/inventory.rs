//! Wire tests for the container and equipment bodies.
//!
//! These are the packets whose codec the extractor refused, all for the same
//! reason: an `ItemStack` is written differently depending on whether it is
//! empty. The point of the vectors here is that boundary. An empty slot is one
//! zero byte with no item id and no component patch behind it, and getting
//! that wrong does not merely draw the wrong item -- every packet sharing the
//! frame is read at the wrong offset afterwards.
//!
//! The bytes are spelled out rather than captured from the jar, because
//! `nix/java/VanillaEncoder.java` has no builder for these yet. Each one is
//! annotated field by field against the `STREAM_CODEC` it comes from, so it is
//! checkable against the same source the generated bodies were read from.

use hyperion_minecraft_proto::{
    Encode, Reader, Writer,
    generated::registry,
    item::{
        DataComponentPatch, ItemStack, Slot,
        nbt::Scanner,
        payload::{CustomName, Text},
    },
    nbt::Tag,
    packets::play::inventory::{
        ContainerSetContent, ContainerSetSlot, EquipmentSlot, SetCursorItem, SetEquipment,
        SetPlayerInventory,
    },
};

fn hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "odd-length hex: {text}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex digit"))
        .collect()
}

fn encode<T: Encode>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

/// `minecraft:diamond_sword`'s network id, which is its position in the item
/// registry and therefore version-specific.
fn diamond_sword() -> i32 {
    i32::try_from(
        registry::ITEM
            .id_of("minecraft:diamond_sword")
            .expect("26.2 has a diamond sword"),
    )
    .expect("registry ids fit in an i32")
}

/// One sword with no components: the smallest non-empty stack there is.
fn sword() -> Slot<'static> {
    Slot::Occupied(ItemStack {
        count: 1,
        item: diamond_sword(),
        components: DataComponentPatch::empty(),
    })
}

#[test]
fn container_set_content_spells_an_empty_slot_as_one_zero_byte() {
    let packet = ContainerSetContent {
        container_id: 3,
        state_id: 7,
        items: vec![Slot::Empty, sword()],
        carried_item: Slot::Empty,
    };

    // 03       containerId, ByteBufCodecs.CONTAINER_ID (a VarInt since 1.21.2)
    // 07       stateId
    // 02       item count, ItemStack.OPTIONAL_LIST_STREAM_CODEC
    // 00         [0] count 0, and nothing else: the empty slot in full
    // 01         [1] count 1
    // c407         item id 964, minecraft:diamond_sword
    // 00 00        DataComponentPatch: nothing added, nothing removed
    // 00       carriedItem, empty
    let expected = hex("0307020001c407000000");
    assert_eq!(diamond_sword(), 964, "item registry moved under the vector");
    assert_eq!(encode(&packet), expected);

    // The empty slot is one byte of the ten, and the occupied one is five.
    assert_eq!(encode(&Slot::Empty), hex("00"));
    assert_eq!(encode(&sword()), hex("01c4070000"));

    let mut reader = Reader::new(&expected);
    let decoded = ContainerSetContent::decode(&mut reader, &Scanner).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(decoded, packet);
}

#[test]
fn a_component_patch_survives_the_round_trip() {
    // A text component is NBT on the wire, so this is also the case that needs
    // the tag scanner: nothing between the component's type id and its value
    // says how long the value is.
    let name = encode(&Tag::String("Excalibur".into()));
    let mut components = DataComponentPatch::empty();
    components
        .set(&CustomName(Text::from_bytes(&name)))
        .expect("custom_name encodes");

    let packet = ContainerSetSlot {
        container_id: 0,
        state_id: 1,
        slot: 36,
        item_stack: Slot::Occupied(ItemStack {
            count: 1,
            item: diamond_sword(),
            components,
        }),
    };

    // 00 01    containerId, stateId
    // 0024     slot 36, a plain big-endian short
    // 01 c407  count 1, minecraft:diamond_sword
    // 01 00    one component added, none removed
    // 06         minecraft:custom_name
    // 080009...  TAG_String "Excalibur", nameless as network NBT always is
    let expected = hex("0001002401c407010006080009457863616c69627572");
    assert_eq!(encode(&packet), expected);

    let mut reader = Reader::new(&expected);
    let decoded = ContainerSetSlot::decode(&mut reader, &Scanner).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(decoded, packet);
}

#[test]
fn the_cursor_has_its_own_packet_now() {
    let packet = SetCursorItem { contents: sword() };
    let expected = hex("01c4070000");
    assert_eq!(encode(&packet), expected);

    let mut reader = Reader::new(&expected);
    let decoded = SetCursorItem::decode(&mut reader, &Scanner).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(decoded, packet);
}

#[test]
fn set_player_inventory_addresses_the_player_not_the_menu() {
    let packet = SetPlayerInventory {
        slot: 8,
        contents: Slot::Empty,
    };
    let expected = hex("0800");
    assert_eq!(encode(&packet), expected);

    let mut reader = Reader::new(&expected);
    let decoded = SetPlayerInventory::decode(&mut reader, &Scanner).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(decoded, packet);
}

#[test]
fn set_equipment_marks_every_entry_but_the_last() {
    let packet = SetEquipment {
        entity: 42,
        slots: vec![
            (EquipmentSlot::MainHand, sword()),
            (EquipmentSlot::Head, Slot::Empty),
        ],
    };

    // 2a       entity id
    // 80       MAINHAND (ordinal 0) with CONTINUE_MASK set: another entry follows
    // 01c40700 00  the sword
    // 05       HEAD (ordinal 5), no mask: this is the last entry
    // 00       empty
    let expected = hex("2a8001c40700000500");
    assert_eq!(encode(&packet), expected);

    let mut reader = Reader::new(&expected);
    let decoded = SetEquipment::decode(&mut reader, &Scanner).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(decoded, packet);
}

#[test]
fn an_equipment_packet_with_no_entries_is_refused() {
    // Vanilla's writer would emit only the entity id, and its reader would then
    // take the next packet's first byte for a slot. Failing here is the only
    // way that cannot happen.
    let packet = SetEquipment {
        entity: 1,
        slots: Vec::new(),
    };
    let mut writer = Writer::new();
    packet.encode(&mut writer).expect_err("no entries to write");
}

#[test]
fn equipment_slot_ordinals_are_declaration_order() {
    // These are read back through EquipmentSlot.VALUES.get(i & 127), so they
    // are positions in net.minecraft.world.entity.EquipmentSlot and not the
    // ids any other packet uses.
    for (index, slot) in EquipmentSlot::ALL.iter().enumerate() {
        let ordinal = u8::try_from(index).expect("eight slots");
        assert_eq!(slot.ordinal(), ordinal);
        assert_eq!(EquipmentSlot::from_ordinal(ordinal), Some(*slot));
    }
    assert_eq!(EquipmentSlot::from_ordinal(8), None);
}

#[test]
fn the_scanner_measures_the_tags_the_item_layer_asks_about() {
    use hyperion_minecraft_proto::item::nbt::NbtScan;

    let tag = encode(&Tag::String("Excalibur".into()));
    // Trailing bytes are expected: a scanner is called mid-packet, not at the
    // end of a tag-sized buffer.
    let mut with_trailer = tag.clone();
    with_trailer.extend_from_slice(&[0xde, 0xad]);
    assert_eq!(Scanner.tag_len(&with_trailer).expect("measure"), tag.len());

    // A bare TAG_End is one byte and means "no tag", which several component
    // shapes can legally carry.
    assert_eq!(Scanner.tag_len(&[0x00, 0x99]).expect("measure"), 1);
}
