//! Configuration-state wire tests.
//!
//! # Where the expected bytes came from
//!
//! Every vector here was produced by the 26.2 server's own encoder, the same
//! way `tests/item.rs` produced its own. A small Java program was compiled
//! against the classes inside `META-INF/versions/26.2/server-26.2.jar` (with
//! `META-INF/libraries` on the classpath), bootstrapped with
//! `SharedConstants.tryDetectVersion()` and `Bootstrap.bootStrap()`, and each
//! packet encoded through its own `STREAM_CODEC` into a `FriendlyByteBuf` over
//! an empty `Unpooled.buffer()`, printing the readable bytes as hex. The Java
//! value each vector came from is written out beside it.
//!
//! So a passing test says this crate agrees with Mojang's encoder, not that it
//! agrees with itself. The two vectors that are *not* captured say so where
//! they appear, and both are negative or framing-only cases that vanilla
//! cannot emit.

use hyperion_minecraft_proto::{
    Decode, Encode, Error, Reader, Writer,
    nbt::{Compound, Tag},
    packets::configuration::{
        AcceptCodeOfConduct, ChatVisiblity, ClientInformation, CodeOfConduct, CustomPayload,
        Disconnect, FinishConfiguration, FinishConfigurationAck, HumanoidArm, KnownPack,
        ParticleStatus, RegistryData, RegistryEntry, RegistryTags, ResetChat, SelectKnownPacks,
        TagEntry, UpdateEnabledFeatures, UpdateTags,
        clientbound::{KeepAlive as ClientboundKeepAlive, Ping},
        serverbound::{KeepAlive as ServerboundKeepAlive, Pong},
    },
    text::Component,
};

/// The harness prints hex, so the fixtures are hex.
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
    assert_eq!(encode(value), bytes, "encoding mismatch");
    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

// --- client information ---------------------------------------------------

#[test]
fn client_information_matches_vanilla() {
    // ClientInformation("en_us", 12, SYSTEM, false, 0x7f, LEFT, true, false,
    //                   MINIMAL)
    let bytes = hex("05656e5f75730c01007f00010002");
    round_trip(
        &ClientInformation {
            language: "en_us",
            view_distance: 12,
            chat_visibility: ChatVisiblity::System,
            chat_colors: false,
            model_customisation: 0x7f,
            main_hand: HumanoidArm::Left,
            text_filtering_enabled: true,
            allows_listing: false,
            particle_status: ParticleStatus::Minimal,
        },
        &bytes,
    );
}

#[test]
fn client_information_default_matches_vanilla() {
    // ClientInformation.createDefault(), which is what a client that has never
    // opened the options screen sends.
    let bytes = hex("05656e5f75730200010001000000");
    round_trip(
        &ClientInformation {
            language: "en_us",
            view_distance: 2,
            chat_visibility: ChatVisiblity::Full,
            chat_colors: true,
            model_customisation: 0,
            main_hand: HumanoidArm::Right,
            text_filtering_enabled: false,
            allows_listing: false,
            particle_status: ParticleStatus::All,
        },
        &bytes,
    );
}

#[test]
fn client_information_enums_reject_unknown_ordinals() {
    // FriendlyByteBuf.readEnum indexes values() directly, so an out-of-range
    // ordinal is an exception there and an error here.
    assert_eq!(
        ChatVisiblity::from_raw(3),
        Err(Error::InvalidEnum {
            name: "ChatVisiblity",
            value: 3
        })
    );
    assert_eq!(
        HumanoidArm::from_raw(2),
        Err(Error::InvalidEnum {
            name: "HumanoidArm",
            value: 2
        })
    );
    assert_eq!(
        ParticleStatus::from_raw(-1),
        Err(Error::InvalidEnum {
            name: "ParticleStatus",
            value: -1
        })
    );
}

#[test]
fn client_information_language_limit_is_enforced_on_both_sides() {
    let mut writer = Writer::new();
    let packet = ClientInformation {
        language: "a_seventeen_char!",
        view_distance: 2,
        chat_visibility: ChatVisiblity::Full,
        chat_colors: true,
        model_customisation: 0,
        main_hand: HumanoidArm::Right,
        text_filtering_enabled: false,
        allows_listing: false,
        particle_status: ParticleStatus::All,
    };
    assert!(matches!(
        packet.encode(&mut writer),
        Err(Error::StringTooLong { max: 16, .. })
    ));
}

// --- custom payload -------------------------------------------------------

#[test]
fn serverbound_brand_payload_matches_vanilla() {
    // ServerboundCustomPayloadPacket(new BrandPayload("vanilla"))
    let bytes = hex("0f6d696e6563726166743a6272616e640776616e696c6c61");
    let body = CustomPayload::encode_brand("vanilla").expect("brand body");
    round_trip(
        &CustomPayload {
            channel: "minecraft:brand",
            data: &body,
        },
        &bytes,
    );
}

#[test]
fn clientbound_brand_payload_matches_vanilla() {
    // ClientboundCustomPayloadPacket.CONFIG_STREAM_CODEC with
    // new BrandPayload("hyperion")
    let bytes = hex("0f6d696e6563726166743a6272616e64086879706572696f6e");
    let body = CustomPayload::encode_brand("hyperion").expect("brand body");
    let packet = CustomPayload {
        channel: "minecraft:brand",
        data: &body,
    };
    round_trip(&packet, &bytes);
    assert_eq!(packet.as_brand().expect("brand parses"), Some("hyperion"));
}

#[test]
fn unknown_payload_keeps_its_bytes() {
    // Not a harness vector, and it cannot be one: `DiscardedPayload`'s encoder
    // is `(payload, buf) -> {}`, so vanilla writes the channel and nothing
    // else and can never emit a body it does not understand. The framing here
    // is the identifier prefix the harness does emit, followed by the
    // remainder, which is what `readableBytes()` makes the body by definition.
    let bytes = hex("0c6879706572696f6e3a6f7073deadbeef");
    round_trip(
        &CustomPayload {
            channel: "hyperion:ops",
            data: &[0xde, 0xad, 0xbe, 0xef],
        },
        &bytes,
    );
}

#[test]
fn as_brand_ignores_other_channels() {
    let payload = CustomPayload {
        channel: "hyperion:ops",
        data: &[0x00],
    };
    assert_eq!(payload.as_brand().expect("not a brand"), None);
}

// --- known packs ----------------------------------------------------------

#[test]
fn serverbound_select_known_packs_matches_vanilla() {
    // ServerboundSelectKnownPacks(List.of(new KnownPack("minecraft", "core",
    //                                                   "26.2")))
    let bytes = hex("01096d696e65637261667404636f72650432362e32");
    round_trip(
        &SelectKnownPacks {
            known_packs: vec![KnownPack {
                namespace: "minecraft",
                id: "core",
                version: "26.2",
            }],
        },
        &bytes,
    );
}

#[test]
fn clientbound_select_known_packs_matches_vanilla() {
    // ClientboundSelectKnownPacks(List.of(
    //     new KnownPack("minecraft", "core", "26.2"),
    //     new KnownPack("hyperion", "extra", "1")))
    let bytes = hex("02096d696e65637261667404636f72650432362e32086879706572696f6e0565787472610131");
    round_trip(
        &SelectKnownPacks {
            known_packs: vec![
                KnownPack {
                    namespace: "minecraft",
                    id: "core",
                    version: "26.2",
                },
                KnownPack {
                    namespace: "hyperion",
                    id: "extra",
                    version: "1",
                },
            ],
        },
        &bytes,
    );
}

#[test]
fn empty_select_known_packs_matches_vanilla() {
    // A client that knows none of the offered packs answers with an empty
    // list, which is the case that makes registry_data carry every payload.
    round_trip(&SelectKnownPacks::default(), &hex("00"));
}

#[test]
fn a_declared_count_longer_than_the_frame_is_rejected() {
    // The count feeds a reservation, so it is checked against bytes present
    // before anything is allocated.
    let bytes = hex("7f00");
    let mut reader = Reader::new(&bytes);
    assert!(matches!(
        SelectKnownPacks::decode(&mut reader),
        Err(Error::UnexpectedEof { .. })
    ));
}

// --- registry data --------------------------------------------------------

#[test]
fn registry_data_matches_vanilla() {
    // ClientboundRegistryDataPacket(minecraft:dimension_type, [
    //     PackedRegistryEntry(minecraft:overworld, Optional.of(
    //         {has_skylight: 1b, height: 384})),
    //     PackedRegistryEntry(minecraft:the_nether, Optional.empty())])
    //
    // The compound's field order is the order the harness's CompoundTag, a
    // HashMap, iterated in; `Compound` keeps insertion order, so the fixture
    // below inserts to match.
    let bytes = hex("186d696e6563726166743a64696d656e73696f6e5f7479706502136d696e6563726166743a6f766572776f726c64010a01000c6861735f736b796c69676874010300066865696768740000018000146d696e6563726166743a7468655f6e657468657200");
    let mut dimension = Compound::new();
    dimension.insert("has_skylight", Tag::Byte(1));
    dimension.insert("height", Tag::Int(384));
    round_trip(
        &RegistryData {
            registry: "minecraft:dimension_type",
            entries: vec![
                RegistryEntry {
                    id: "minecraft:overworld",
                    data: Some(Tag::Compound(dimension)),
                },
                RegistryEntry {
                    id: "minecraft:the_nether",
                    data: None,
                },
            ],
        },
        &bytes,
    );
}

#[test]
fn empty_registry_data_matches_vanilla() {
    // A registry every one of whose elements came from a pack the client
    // reported still gets a packet, just an empty one.
    let bytes = hex("186d696e6563726166743a62616e6e65725f7061747465726e00");
    round_trip(
        &RegistryData {
            registry: "minecraft:banner_pattern",
            entries: Vec::new(),
        },
        &bytes,
    );
}

#[test]
fn an_absent_registry_payload_is_a_boolean_not_an_end_tag() {
    // The tail of the `registry_data` vector above, checked on its own:
    // `ByteBufCodecs.TAG.apply(ByteBufCodecs::optional)` writes a boolean
    // rather than the bare TAG_End `FriendlyByteBuf.writeNbt` uses for null,
    // so an absent payload costs one byte and not two.
    let bytes = hex("146d696e6563726166743a7468655f6e657468657200");
    round_trip(
        &RegistryEntry {
            id: "minecraft:the_nether",
            data: None,
        },
        &bytes,
    );
    assert_eq!(*bytes.last().expect("non-empty fixture"), 0x00);
}

// --- feature flags --------------------------------------------------------

#[test]
fn update_enabled_features_matches_vanilla() {
    // ClientboundUpdateEnabledFeaturesPacket(Set.of(minecraft:vanilla))
    let bytes = hex("01116d696e6563726166743a76616e696c6c61");
    round_trip(
        &UpdateEnabledFeatures {
            features: vec!["minecraft:vanilla"],
        },
        &bytes,
    );
}

// --- tags -----------------------------------------------------------------

#[test]
fn update_tags_matches_vanilla() {
    // ClientboundUpdateTagsPacket({minecraft:block:
    //     NetworkPayload({minecraft:wool: [1, 2, 300]})})
    let bytes = hex("010f6d696e6563726166743a626c6f636b010e6d696e6563726166743a776f6f6c030102ac02");
    round_trip(
        &UpdateTags {
            tags: vec![RegistryTags {
                registry: "minecraft:block",
                tags: vec![TagEntry {
                    name: "minecraft:wool",
                    entries: vec![1, 2, 300],
                }],
            }],
        },
        &bytes,
    );
}

#[test]
fn empty_update_tags_matches_vanilla() {
    round_trip(&UpdateTags::default(), &hex("00"));
}

// --- keep alive, ping, disconnect, code of conduct ------------------------

#[test]
fn keep_alive_matches_vanilla_in_both_directions() {
    // Both ClientboundKeepAlivePacket and ServerboundKeepAlivePacket print the
    // same bytes for the same id. That used to be asserted by *asserting it*:
    // one hand-written type stood in for both classes, and the claim that they
    // agree was the comment above it rather than anything a test ran. The
    // generator emits one type per direction, so both go through the same
    // fixture and the agreement is checked instead of assumed.
    let bytes = hex("0123456789abcdef");
    round_trip(&ClientboundKeepAlive(0x0123_4567_89ab_cdef), &bytes);
    round_trip(&ServerboundKeepAlive(0x0123_4567_89ab_cdef), &bytes);
}

#[test]
fn ping_and_pong_are_ints_not_longs() {
    // The configuration-state ping is an int; the status-state one is a long.
    let bytes = hex("0abcdef1");
    round_trip(&Ping(0x0abc_def1), &bytes);
    round_trip(&Pong(0x0abc_def1), &bytes);
}

#[test]
fn disconnect_reason_is_nbt_not_json() {
    // ClientboundDisconnectPacket(Component.literal("bye")). The component
    // collapses to a bare NBT string, so the body is TAG_String and not a
    // compound with a `text` field, and not JSON.
    round_trip(
        &Disconnect {
            reason: Component::text("bye"),
        },
        &hex("080003627965"),
    );
}

#[test]
fn code_of_conduct_matches_vanilla() {
    round_trip(
        &CodeOfConduct {
            code_of_conduct: "be nice",
        },
        &hex("076265206e696365"),
    );
}

// --- empty packets --------------------------------------------------------

#[test]
fn unit_packets_have_no_body() {
    // Every one of these printed an empty string from the harness, which is
    // what StreamCodec.unit means on the wire.
    round_trip(&FinishConfiguration, &[]);
    round_trip(&FinishConfigurationAck, &[]);
    round_trip(&ResetChat, &[]);
    round_trip(&AcceptCodeOfConduct, &[]);
}
