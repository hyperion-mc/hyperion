//! Item and data component tests.
//!
//! # Where the expected bytes came from
//!
//! Every vector marked `captured` was produced by the 26.2 server's own
//! encoder. A small Java program was compiled against the classes inside
//! `META-INF/versions/26.2/server-26.2.jar` (with `META-INF/libraries` on the
//! classpath), bootstrapped with `SharedConstants.tryDetectVersion()` and
//! `Bootstrap.bootStrap()`, and encoded values through
//! `DataComponentPatch.STREAM_CODEC` and `ItemStackTemplate.STREAM_CODEC`
//! into a `RegistryFriendlyByteBuf`, printing the buffer as hex.
//!
//! It never constructs an `ItemStack`. In 26.x an item's default components
//! come from the datapack layer, and binding them fails without a loaded pack
//! (`Components not bound yet`, then `Missing tag minecraft:is_fire`). Nothing
//! important is lost: `ItemStack.OPTIONAL_STREAM_CODEC` is a count, an item id
//! and a patch, and only the patch has a codec worth capturing. The two vectors
//! marked `transcribed` are hand-assembled from that framing and from a codec
//! over a datapack registry the harness cannot populate; they are called out
//! individually.
//!
//! Item ids in these vectors are the real ones the harness printed for 26.2:
//! `stone` 1, `gold_ingot` 936, `iron_pickaxe` 961, `diamond_sword` 964,
//! `stick` 974, `paper` 1057, `bundle` 1065, `diamond` 926, `emerald` 927,
//! `brick` 1054.
//!
//! # What is not covered
//!
//! 85 of the 111 component types have a captured value here. The other 26 are
//! ones the harness cannot build, because their codecs reach a registry that
//! only exists once a datapack is loaded:
//!
//! `enchantments`, `stored_enchantments`, `damage_type`, `trim`,
//! `provides_trim_material`, `instrument`, `jukebox_playable`,
//! `banner_patterns`, `provides_banner_patterns`, `painting/variant`,
//! `villager/variant`, `wolf/variant`, `wolf/sound_variant`, `pig/variant`,
//! `pig/sound_variant`, `cow/variant`, `cow/sound_variant`,
//! `chicken/variant`, `chicken/sound_variant`, `zombie_nautilus/variant`,
//! `frog/variant`, `cat/variant`, `cat/sound_variant`, plus `can_break`,
//! `lock` and `container_loot`.
//!
//! Some of those are safer than the list suggests. `can_break` shares its
//! shape with the captured `can_place_on`, `stored_enchantments` shares its
//! shape with `enchantments`, `lock` and `container_loot` are plain tags like
//! the captured `map_decorations`, and the twelve animal variants are bare
//! registry ids identical to the fifteen that are captured. The ones with real
//! residual risk are the holder-with-inline-value shapes: `trim`, `instrument`,
//! `jukebox_playable`, `banner_patterns` and `painting/variant`.

use hyperion_minecraft_proto::{
    Encode, Error, Reader, Writer,
    generated::registry,
    item::{
        ComponentType, DataComponentPatch, ItemStack, Slot,
        nbt::NbtScan,
        payload::{CustomData, CustomName, Damage, ItemModel, Lore, MaxStackSize, Unbreakable},
        shape::MAX_DEPTH,
    },
};

/// A structural NBT scanner, for tests only.
///
/// The crate takes tag measurement as a parameter so it does not have to carry
/// an NBT implementation; the real one comes from the NBT module. This is the
/// smallest thing that satisfies the contract, and it exists so these tests do
/// not have to wait on that module to land.
struct TestNbt;

impl TestNbt {
    /// Length of the payload of a tag of type `kind` starting at `bytes`.
    fn payload_len(kind: u8, bytes: &[u8]) -> Option<usize> {
        let fixed = |n: usize| (bytes.len() >= n).then_some(n);
        match kind {
            0 => Some(0),
            1 => fixed(1),
            2 => fixed(2),
            3 | 5 => fixed(4),
            4 | 6 => fixed(8),
            7 => Self::array(bytes, 1),
            8 => {
                let len = usize::from(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]));
                (bytes.len() >= 2 + len).then_some(2 + len)
            }
            9 => {
                let element = *bytes.first()?;
                let count = i32::from_be_bytes(bytes.get(1..5)?.try_into().ok()?);
                let mut at = 5;
                for _ in 0..count.max(0) {
                    at += Self::payload_len(element, bytes.get(at..)?)?;
                }
                Some(at)
            }
            10 => {
                let mut at = 0;
                loop {
                    let entry = *bytes.get(at)?;
                    at += 1;
                    if entry == 0 {
                        return Some(at);
                    }
                    let name =
                        usize::from(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
                    at += 2 + name;
                    at += Self::payload_len(entry, bytes.get(at..)?)?;
                }
            }
            11 => Self::array(bytes, 4),
            12 => Self::array(bytes, 8),
            _ => None,
        }
    }

    fn array(bytes: &[u8], stride: usize) -> Option<usize> {
        let count = i32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?);
        let total = 4 + usize::try_from(count).ok()? * stride;
        (bytes.len() >= total).then_some(total)
    }
}

impl NbtScan for TestNbt {
    fn tag_len(&self, bytes: &[u8]) -> hyperion_minecraft_proto::Result<usize> {
        let kind = *bytes.first().ok_or(Error::UnexpectedEof {
            needed: 1,
            available: 0,
        })?;
        Self::payload_len(kind, &bytes[1..])
            .map(|len| len + 1)
            .ok_or(Error::InvalidUtf8)
    }
}

/// Decode hex, so vectors read the way the harness printed them.
fn hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "odd-length hex: {text}");
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("hex digit"))
        .collect()
}

/// Decode a patch and assert it re-encodes to exactly the bytes it came from.
///
/// The buffer is leaked so the patch can be returned for further inspection;
/// these are one-shot test cases, so the alternative is threading a buffer
/// through every caller for no gain.
fn round_trip(vector: &str) -> DataComponentPatch<'static> {
    let bytes: &'static [u8] = hex(vector).leak();
    let mut reader = Reader::new(bytes);
    let patch = DataComponentPatch::decode(&mut reader, &TestNbt).expect("decode");
    reader.finish().expect("patch consumed every byte");

    let mut writer = Writer::new();
    patch.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), bytes, "re-encode differed for {vector}");
    patch
}

#[test]
fn component_types_agree_with_the_generated_registry() {
    let table = registry::DATA_COMPONENT_TYPE;
    assert_eq!(ComponentType::all().len(), 111);
    assert_eq!(table.entries.len(), ComponentType::all().len());
    for (index, kind) in ComponentType::all().iter().enumerate() {
        let id = i32::try_from(index).unwrap();
        assert_eq!(kind.id(), id, "discriminant is not its registry position");
        assert_eq!(ComponentType::from_id(id), Some(*kind));
        assert_eq!(kind.name(), table.entries[index]);
    }
    assert_eq!(ComponentType::from_id(111), None);
    assert_eq!(ComponentType::from_id(-1), None);
}

#[test]
fn empty_patch_is_two_zero_counts() {
    // captured: DataComponentPatch.builder().build()
    let patch = round_trip("0000");
    assert!(patch.is_empty());
}

#[test]
fn custom_name_is_a_bare_string_tag() {
    // captured: set(CUSTOM_NAME, Component.literal("Excalibur"))
    let patch = round_trip("010006080009457863616c69627572");
    let name: CustomName<'_> = patch.get(&TestNbt).expect("present").expect("parse");
    // Type 8 is TAG_String, then a two-byte length and the UTF-8 body. The
    // tag is nameless, which is what "network NBT" means since 1.20.2.
    assert_eq!(name.0.bytes(), hex("080009457863616c69627572"));
}

#[test]
fn damage_is_a_var_int() {
    // captured: set(DAMAGE, 42)
    let patch = round_trip("0100032a");
    assert_eq!(
        patch
            .get::<Damage>(&TestNbt)
            .expect("present")
            .expect("parse"),
        Damage(42)
    );
}

#[test]
fn lore_is_a_list_of_text_components() {
    // captured: set(LORE, [literal("first"), literal("second")])
    let patch = round_trip("01000b0208000566697273740800067365636f6e64");
    let lore: Lore<'_> = patch.get(&TestNbt).expect("present").expect("parse");
    assert_eq!(lore.0.len(), 2);
    assert_eq!(lore.0[0].bytes(), hex("0800056669727374"));
    assert_eq!(lore.0[1].bytes(), hex("0800067365636f6e64"));
}

#[test]
fn custom_data_is_one_compound_tag() {
    // captured: set(CUSTOM_DATA, {hyperion: "marker"})
    let patch = round_trip("0100000a0800086879706572696f6e00066d61726b657200");
    let data: CustomData<'_> = patch.get(&TestNbt).expect("present").expect("parse");
    assert_eq!(data.0, hex("0a0800086879706572696f6e00066d61726b657200"));
}

#[test]
fn three_components_in_one_patch() {
    // captured: set(ITEM_MODEL, "hyperion:wand"), set(UNBREAKABLE), set(MAX_STACK_SIZE, 1)
    let patch = round_trip("03000a0d6879706572696f6e3a77616e64040101");
    assert_eq!(patch.added().len(), 3);
    assert_eq!(
        patch
            .get::<ItemModel<'_>>(&TestNbt)
            .expect("present")
            .expect("parse")
            .0,
        "hyperion:wand"
    );
    // A marker component occupies zero bytes, so its span is empty rather than
    // absent -- the distinction the patch has to keep.
    assert_eq!(patch.raw(ComponentType::Unbreakable), Some(&[][..]));
    assert_eq!(
        patch
            .get::<Unbreakable>(&TestNbt)
            .expect("present")
            .expect("parse"),
        Unbreakable
    );
    assert_eq!(
        patch
            .get::<MaxStackSize>(&TestNbt)
            .expect("present")
            .expect("parse"),
        MaxStackSize(1)
    );
}

#[test]
fn removals_are_a_separate_group() {
    // captured: remove(ATTRIBUTE_MODIFIERS), remove(RARITY)
    let patch = round_trip("0002100c");
    assert!(patch.added().is_empty());
    assert_eq!(patch.removed(), [
        ComponentType::AttributeModifiers,
        ComponentType::Rarity
    ]);

    // captured: set(DAMAGE, 7), remove(RARITY)
    let mixed = round_trip("010103070c");
    assert_eq!(mixed.added().len(), 1);
    assert_eq!(mixed.removed(), [ComponentType::Rarity]);
}

#[test]
fn holder_sets_bias_their_count_by_one() {
    // captured: set(REPAIRABLE, HolderSet.direct(diamond, emerald))
    // The marker is 3 for two entries, leaving 0 to mean "a tag name follows".
    let patch = round_trip("010021039e079f07");
    assert_eq!(
        patch.raw(ComponentType::Repairable),
        Some(&hex("039e079f07")[..])
    );
}

#[test]
fn maps_and_registry_id_lists() {
    // captured: set(BLOCK_STATE, {axis: "z"})
    round_trip("01004c010461786973017a");
    // captured: set(POT_DECORATIONS, [brick, diamond, brick, emerald])
    round_trip("01004a049e089e079e089f07");
}

#[test]
fn attribute_modifiers_carry_a_display_dispatch() {
    // captured: set(ATTRIBUTE_MODIFIERS, attack_damage +3.5 in mainhand)
    // Trailing `00` is the display selector for `default`, which carries no
    // payload; getting that wrong would swallow or invent bytes.
    round_trip("01001001030d6879706572696f6e3a74657374400c000000000000000100");
}

#[test]
fn a_stack_nested_in_a_component_recurses_into_another_patch() {
    // captured: set(BUNDLE_CONTENTS, [gold_ingot x7 named "loot"])
    let patch = round_trip("01003201a807070100060800046c6f6f74");
    // captured separately through ItemStackTemplate.STREAM_CODEC, and it is
    // byte-identical to the element inside the list above -- which is how the
    // item-then-count field order of the template form is confirmed.
    assert_eq!(
        patch.raw(ComponentType::BundleContents),
        Some(&hex("01a807070100060800046c6f6f74")[..])
    );
}

#[test]
fn an_unmodelled_component_survives_byte_for_byte() {
    // captured: set(BUNDLE_CONTENTS, ...). Nothing in `payload` models bundle
    // contents, so this only works because the shape table can delimit it.
    // This is the property that keeps a proxy from corrupting an inventory.
    let vector = "01003201a807070100060800046c6f6f74";
    let bytes = hex(vector);
    let mut reader = Reader::new(&bytes);
    let patch = DataComponentPatch::decode(&mut reader, &TestNbt).expect("decode");
    reader.finish().expect("consumed");

    let mut writer = Writer::new();
    patch.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), bytes.as_slice());
}

#[test]
fn every_component_type_has_a_usable_shape() {
    // Not a claim that each layout is right -- that is what the captured
    // vectors are for. This asserts the weaker thing that keeps the table
    // honest: no type is missing an entry, and none of them panics.
    for kind in ComponentType::all() {
        let shape = kind.shape();
        let mut reader = Reader::new(&[]);
        // Empty input: a zero-length shape succeeds, everything else must
        // report end of input rather than misbehave.
        drop(shape.skip(&mut reader, &TestNbt));
    }
}

#[test]
fn an_unknown_component_id_is_rejected_rather_than_guessed() {
    // 0x7f is past the end of the registry. There is no length prefix to skip
    // over, so continuing would desynchronise the rest of the packet; refusing
    // is the only correct answer.
    let bytes = hex("01007f00");
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        DataComponentPatch::decode(&mut reader, &TestNbt),
        Err(Error::InvalidEnum {
            name: "data component type",
            value: 127
        })
    );
}

#[test]
fn a_slot_with_a_zero_count_is_empty() {
    // transcribed from ItemStack.OPTIONAL_STREAM_CODEC: a count of zero ends
    // the read, with no item id or components after it.
    let bytes = hex("00");
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        Slot::decode(&mut reader, &TestNbt).expect("decode"),
        Slot::Empty
    );
    reader.finish().expect("consumed");

    let mut writer = Writer::new();
    Slot::Empty.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), &bytes[..]);

    // The same bytes are an error where a non-empty stack is required.
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        ItemStack::decode(&mut reader, &TestNbt),
        Err(Error::EmptyItemStack)
    );
}

#[test]
fn an_occupied_slot_is_count_then_item_then_patch() {
    // transcribed: ItemStack.OPTIONAL_STREAM_CODEC framing (count, item id,
    // patch) around the captured `damage_42` patch, with the harness-reported
    // id for iron_pickaxe (961 -> c107).
    let bytes = hex("01c1070100032a");
    let mut reader = Reader::new(&bytes);
    let Slot::Occupied(stack) = Slot::decode(&mut reader, &TestNbt).expect("decode") else {
        panic!("expected an occupied slot");
    };
    reader.finish().expect("consumed");
    assert_eq!(stack.count, 1);
    assert_eq!(stack.item, 961);
    assert_eq!(
        stack
            .components
            .get::<Damage>(&TestNbt)
            .expect("present")
            .expect("parse"),
        Damage(42)
    );

    let mut writer = Writer::new();
    Slot::Occupied(stack).encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), &bytes[..]);
}

#[test]
fn a_component_set_from_a_structure_matches_the_captured_bytes() {
    // Building the same patch the harness built must produce the harness's
    // bytes; that is the encoder being checked against Mojang rather than
    // against this crate's own decoder.
    let mut patch = DataComponentPatch::empty();
    patch.set(&Damage(42)).expect("encode");
    let mut writer = Writer::new();
    patch.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), hex("0100032a"));
}

#[test]
fn setting_a_component_clears_a_pending_removal() {
    let mut patch = DataComponentPatch::empty();
    patch.remove(ComponentType::Damage);
    patch.set(&Damage(1)).expect("encode");
    assert!(patch.removed().is_empty());
    assert_eq!(patch.added().len(), 1);

    patch.remove(ComponentType::Damage);
    assert!(patch.added().is_empty());
    assert_eq!(patch.removed(), [ComponentType::Damage]);
}

#[test]
fn nesting_stops_before_the_stack_does() {
    // Bundles inside bundles: each level is a patch holding one bundle_contents
    // holding one template. Deep enough and the walker must give up rather than
    // overflow, because a client chooses this depth, not the server.
    let mut bytes = Vec::new();
    let depth = usize::try_from(MAX_DEPTH).unwrap() * 2;
    for _ in 0..depth {
        // one added, none removed; type 50 (bundle_contents); a list of one
        // template; item 1, count 1; then the next patch.
        bytes.extend_from_slice(&hex("010032010101"));
    }
    bytes.extend_from_slice(&hex("0000"));

    let mut reader = Reader::new(&bytes);
    assert_eq!(
        DataComponentPatch::decode(&mut reader, &TestNbt),
        Err(Error::DepthLimitExceeded(MAX_DEPTH))
    );
}

#[test]
fn a_count_larger_than_the_buffer_is_refused_immediately() {
    // A four-byte varint claiming a huge list. Nothing here should try to
    // allocate for it before noticing the buffer cannot hold that many.
    let bytes = hex("0100320affffff7f");
    let mut reader = Reader::new(&bytes);
    assert!(matches!(
        DataComponentPatch::decode(&mut reader, &TestNbt),
        Err(Error::UnexpectedEof { .. })
    ));
}

/// Every component the harness could build from built-in registries, in one
/// patch: 84 set and 2 removed.
///
/// captured: a second Java program that sets each of these through
/// `DataComponentPatch.Builder` and encodes the result once.
///
/// This is the strongest check in the file. Component values are not
/// length-prefixed, so one wrong shape does not mislabel one value -- it moves
/// the read head and every component after it decodes as nonsense or fails.
/// Getting 84 in a row right, and re-encoding to the same 627 bytes, is hard to
/// do by accident.
const KITCHEN_SINK: &[&str] = &[
    "5402000a0800016b00017600011002fa010303040501003f000000060800046e616d6507",
    "3f666666090800046974656d0a03613a620b010800016c0c030e01010301090000000011",
    "013fc000000201000101730100112233120002030c1304141501160a0017043e99999a01",
    "183fcccccd01bb0501030341000000048f070219a8070300001a402000000103613a631c",
    "010201014080000001013f80000002001d013ecccccd1f0a21029e07222303613a642401",
    "034000000028010629052b052c004455662d007788992e0c30003101a8070300003201a8",
    "0703000033000100aabbcc010901c80100010100010462726577343fa000003501003c36",
    "0104706167650037057469746c650006617574686f720101080001700001390a003a640a",
    "0800016b000176003b0a0800016b000176003f024301136d696e6563726166743a6f7665",
    "72776f726c64000000400000300201440202000000010000000201000000030100450201",
    "04010000000900000146000106706c617965720000000000004703613a65490e4a049e08",
    "9e089e089e074b020001a8070300004c01046178697301794d01640a0800016b00017600",
    "0a144ea807030000518f076d096e0a6c06550b5601570258015980025a055b075c015d63",
    "6602680369021e3f000000404000003f80000040a000003dcccccd3e80000020048f0700",
    "0103613a6601026400000001018f07253e4ccccd3f4000000142b40000003f8000003f00",
    "00003f800000400000003f00000000018f0700260100018f0700270502010a3f80000040",
    "0000000001033f0000003e8000003fc0000040200000018f07003c010a0800016b000176",
    "001b012f0a00420900000000004f50",
];

#[test]
fn eighty_four_components_in_one_patch() {
    let vector = KITCHEN_SINK.concat();
    let patch = round_trip(&vector);
    assert_eq!(patch.added().len(), 84);
    assert_eq!(patch.removed(), [
        ComponentType::Lock,
        ComponentType::ContainerLoot
    ]);

    // Spot-check both ends, so a walk that happened to land back on its feet
    // after a mistake still shows up.
    assert_eq!(
        patch
            .get::<CustomData<'_>>(&TestNbt)
            .expect("present")
            .expect("parse")
            .0,
        hex("0a0800016b00017600")
    );
    assert_eq!(
        patch
            .get::<Damage>(&TestNbt)
            .expect("present")
            .expect("parse"),
        Damage(3)
    );
    assert_eq!(
        patch.raw(ComponentType::AxolotlVariant),
        Some(&hex("02")[..])
    );
}
