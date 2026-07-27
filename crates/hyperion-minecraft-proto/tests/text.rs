//! Text component tests.
//!
//! The expected bytes came out of the Minecraft 26.2 server jar. A harness on
//! the classpath of the bundled `server-26.2.jar` built each component with
//! Mojang's own classes, ran `ComponentSerialization.CODEC.encodeStart` against
//! `NbtOps.INSTANCE` — which is what `fromCodecWithRegistries` does before it
//! reaches the wire — and printed `NbtIo.writeAnyTag`'s output as hex.
//!
//! Comparing raw bytes only works one way round. `CompoundTag` is a `HashMap`,
//! so the field order in a vanilla encoding is whatever the hash table
//! happened to produce and no implementation can reproduce it on purpose. So
//! [`agrees_with_vanilla`] compares decoded values instead: the vanilla bytes
//! must decode to the expected component, and this crate's encoding of that
//! component must be the same NBT document as the vanilla bytes. Field order
//! is the only freedom that leaves.

use std::borrow::Cow;

use hyperion_minecraft_proto::{
    Decode, Encode, Error, Reader, Writer,
    nbt::{Compound, Tag},
    text::{
        Argument, ClickEvent, Component, Contents, DataSource, HoverEvent, NamedColor, NbtContents,
        ObjectInfo, Score, Style, TextColor, Translatable,
    },
};

fn vanilla(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "odd-length hex: {hex}");
    (0..hex.len() / 2)
        .map(|index| {
            u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex digit pair")
        })
        .collect()
}

fn decode_tag(bytes: &[u8]) -> Tag<'_> {
    let mut reader = Reader::new(bytes);
    let tag = Tag::decode(&mut reader).expect("decode NBT");
    reader.finish().expect("tag fully consumed");
    tag
}

/// Assert this crate reads vanilla's bytes as `expected`, and writes `expected`
/// as the same NBT document vanilla wrote.
fn agrees_with_vanilla(expected: &Component<'_>, hex: &str) {
    let bytes = vanilla(hex);
    let theirs = decode_tag(&bytes);

    let decoded = Component::from_tag(&theirs).expect("decode component");
    assert_eq!(&decoded, expected, "decoded {hex}");

    let mut writer = Writer::new();
    expected.encode(&mut writer).expect("encode");
    let mine = writer.into_vec();
    assert_eq!(
        decode_tag(&mine),
        theirs,
        "encoded a different document than vanilla: {mine:02X?}"
    );
}

fn styled<'a>(text: &'a str, style: Style<'a>) -> Component<'a> {
    Component::text(text).with_style(style)
}

const fn string(value: &str) -> Tag<'_> {
    Tag::String(Cow::Borrowed(value))
}

fn compound<'a>(entries: impl IntoIterator<Item = (&'a str, Tag<'a>)>) -> Tag<'a> {
    Tag::Compound(
        entries
            .into_iter()
            .map(|(name, value)| (Cow::Borrowed(name), value))
            .collect(),
    )
}

fn list<'a>(elements: impl IntoIterator<Item = Tag<'a>>) -> Tag<'a> {
    Tag::List(elements.into_iter().collect())
}

// --- the collapsed form ---------------------------------------------------

#[test]
fn a_bare_literal_is_a_string_not_a_compound() {
    // `tryCollapseToString`: a literal with no style and no children is a bare
    // TAG_String. Five bytes, and no `text` field anywhere.
    agrees_with_vanilla(&Component::text("hi"), "0800026869");
    agrees_with_vanilla(&Component::text(""), "080000");
}

#[test]
fn a_styled_literal_is_a_compound() {
    agrees_with_vanilla(
        &styled("hi", Style {
            bold: Some(true),
            ..Style::new()
        }),
        "0A0800047465787400026869010004626F6C640100",
    );
}

#[test]
fn a_list_is_its_head_with_the_tail_appended() {
    // The decode-only third shape. `createFromList` copies the first element
    // and appends the rest to it, and nothing ever encodes back to a list.
    let component = Component::from_tag(&list([string("a"), string("b")])).expect("decode");
    assert_eq!(component, Component::text("a").append(Component::text("b")));
}

#[test]
fn an_empty_extra_list_is_rejected() {
    // ExtraCodecs.nonEmptyList, rather than treating it as no children.
    let tag = compound([("text", string("a")), ("extra", list([]))]);
    assert_eq!(Component::from_tag(&tag), Err(Error::MissingField("extra")));
}

// --- contents -------------------------------------------------------------

#[test]
fn children_carry_their_own_style() {
    agrees_with_vanilla(
        &styled("a", Style {
            bold: Some(true),
            ..Style::new()
        })
        .append(styled("b", Style {
            italic: Some(true),
            ..Style::new()
        })),
        "0A09000565787472610A00000001080004746578740001620100066974616C6963010008000474657874000161010004626F6C640100",
    );
}

#[test]
fn extra_holds_collapsed_children() {
    agrees_with_vanilla(
        &Component::text("a").append(Component::text("b")),
        "0A090005657874726108000000010001620800047465787400016100",
    );
}

#[test]
fn translatable_arguments_collapse_to_primitives() {
    // A component argument that collapses to a string is written as that
    // string: ARG_CODEC's component branch runs the same collapse the top
    // level does, and its primitive branch reads it back as a string rather
    // than as a component.
    agrees_with_vanilla(
        &Component::from_contents(Contents::Translatable(Translatable {
            key: Cow::Borrowed("chat.type.text"),
            fallback: None,
            with: vec![
                Argument::String(Cow::Borrowed("Notch")),
                Argument::String(Cow::Borrowed("hi")),
            ],
        })),
        "0A09000477697468080000000200054E6F746368000268690800097472616E736C617465000E636861742E747970652E7465787400",
    );
}

#[test]
fn translatable_fallback_round_trips() {
    agrees_with_vanilla(
        &Component::from_contents(Contents::Translatable(Translatable {
            key: Cow::Borrowed("k"),
            fallback: Some(Cow::Borrowed("fb")),
            with: Vec::new(),
        })),
        "0A08000866616C6C6261636B000266620800097472616E736C61746500016B00",
    );
}

#[test]
fn keybind_round_trips() {
    agrees_with_vanilla(
        &Component::from_contents(Contents::Keybind(Cow::Borrowed("key.jump"))),
        "0A0800076B657962696E6400086B65792E6A756D7000",
    );
}

#[test]
fn score_nests_under_its_own_key() {
    // The one content type whose fields are not flat: MAP_CODEC is
    // `INNER_CODEC.fieldOf("score")`.
    agrees_with_vanilla(
        &Component::from_contents(Contents::Score(Score {
            name: Cow::Borrowed("Notch"),
            objective: Cow::Borrowed("obj"),
        })),
        "0A0A000573636F72650800046E616D6500054E6F7463680800096F626A65637469766500036F626A0000",
    );
}

#[test]
fn selector_separator_round_trips() {
    agrees_with_vanilla(
        &Component::from_contents(Contents::Selector {
            selector: Cow::Borrowed("@a"),
            separator: Some(Box::new(Component::text(", "))),
        }),
        "0A08000873656C6563746F7200024061080009736570617261746F7200022C2000",
    );
}

#[test]
fn nbt_source_is_flattened_alongside_the_path() {
    // `plain` is absent because it is false, which is its default; `interpret`
    // is present because it is not. `plain` itself is new in 26.x.
    agrees_with_vanilla(
        &Component::from_contents(Contents::Nbt(NbtContents {
            path: Cow::Borrowed("foo.bar"),
            interpret: true,
            plain: false,
            separator: None,
            source: DataSource::Storage(Cow::Borrowed("minecraft:store")),
        })),
        "0A0800036E62740007666F6F2E626172010009696E746572707265740108000773746F72616765000F6D696E6563726166743A73746F726500",
    );
}

#[test]
fn an_object_component_omits_the_default_atlas() {
    // `object` is the content type the older documentation does not have at
    // all. `atlas` defaults to minecraft:blocks and vanishes when it matches.
    agrees_with_vanilla(
        &Component::from_contents(Contents::Object {
            object: ObjectInfo::Atlas {
                atlas: Cow::Borrowed("minecraft:blocks"),
                sprite: Cow::Borrowed("minecraft:stone"),
            },
            fallback: None,
        }),
        "0A080006737072697465000F6D696E6563726166743A73746F6E6500",
    );
}

#[test]
fn contents_with_no_recognised_field_are_rejected() {
    let tag = compound([("bold", Tag::Byte(1))]);
    assert_eq!(
        Component::from_tag(&tag),
        Err(Error::NoMatchingCodec("component contents"))
    );
}

// --- style ----------------------------------------------------------------

#[test]
fn a_named_colour_is_its_name() {
    agrees_with_vanilla(
        &styled("hi", Style {
            color: Some(TextColor::Named(NamedColor::Red)),
            ..Style::new()
        }),
        "0A080005636F6C6F720003726564080004746578740002686900",
    );
}

#[test]
fn an_rgb_colour_is_uppercase_hex() {
    // `String.format(Locale.ROOT, "#%06X", value)`.
    agrees_with_vanilla(
        &styled("hi", Style {
            color: Some(TextColor::Rgb(0x0012_3456)),
            ..Style::new()
        }),
        "0A080005636F6C6F72000723313233343536080004746578740002686900",
    );
}

#[test]
fn every_named_colour_survives_its_name() {
    for color in NamedColor::ALL {
        assert_eq!(NamedColor::parse(color.as_str()), Some(color));
        assert_eq!(
            TextColor::parse(color.as_str()),
            Ok(TextColor::Named(color))
        );
    }
    assert_eq!(NamedColor::Gold.rgb(), 0x00FF_AA00);
}

#[test]
fn an_unknown_colour_is_rejected() {
    assert!(TextColor::parse("puce").is_err());
    // Out of 24 bits.
    assert!(TextColor::parse("#1123456").is_err());
}

#[test]
fn all_five_flags_round_trip_including_the_false_ones() {
    // `Style` distinguishes unset from false, so `italic: 0b` has to survive.
    agrees_with_vanilla(
        &styled("x", Style {
            bold: Some(true),
            italic: Some(false),
            underlined: Some(true),
            strikethrough: Some(false),
            obfuscated: Some(true),
            ..Style::new()
        }),
        "0A01000A756E6465726C696E65640108000474657874000178010004626F6C640101000D737472696B657468726F756768000100066974616C69630001000A6F6266757363617465640100",
    );
}

#[test]
fn shadow_colour_is_a_packed_argb_int() {
    agrees_with_vanilla(
        &styled("x", Style {
            shadow_color: Some(0x1122_3344),
            ..Style::new()
        }),
        "0A03000C736861646F775F636F6C6F72112233440800047465787400017800",
    );
}

#[test]
fn insertion_and_font_round_trip() {
    agrees_with_vanilla(
        &styled("x", Style {
            insertion: Some(Cow::Borrowed("ins")),
            font: Some(Cow::Borrowed("minecraft:alt")),
            ..Style::new()
        }),
        "0A080009696E73657274696F6E0003696E7308000474657874000178080004666F6E74000D6D696E6563726166743A616C7400",
    );
}

// --- events ---------------------------------------------------------------

#[test]
fn click_events_round_trip() {
    agrees_with_vanilla(
        &styled("go", Style {
            click_event: Some(ClickEvent::RunCommand(Cow::Borrowed("/say hi"))),
            ..Style::new()
        }),
        "0A0A000B636C69636B5F6576656E74080006616374696F6E000B72756E5F636F6D6D616E64080007636F6D6D616E6400072F73617920686900080004746578740002676F00",
    );
    agrees_with_vanilla(
        &styled("go", Style {
            click_event: Some(ClickEvent::OpenUrl(Cow::Borrowed("https://example.com"))),
            ..Style::new()
        }),
        "0A0A000B636C69636B5F6576656E74080006616374696F6E00086F70656E5F75726C08000375726C001368747470733A2F2F6578616D706C652E636F6D00080004746578740002676F00",
    );
}

#[test]
fn open_file_cannot_appear_on_the_wire() {
    // `Action.CODEC` is `UNSAFE_CODEC.validate(filterForSerialization)`, and
    // `validate` runs on decode too, so `open_file` is not a click event a
    // connection can carry in either direction.
    let tag = compound([
        ("text", string("x")),
        (
            "click_event",
            compound([("action", string("open_file")), ("path", string("/"))]),
        ),
    ]);
    assert!(matches!(
        Component::from_tag(&tag),
        Err(Error::UnknownVariant { .. })
    ));
}

#[test]
fn change_page_rejects_zero() {
    // ExtraCodecs.POSITIVE_INT.
    let tag = compound([
        ("text", string("x")),
        (
            "click_event",
            compound([("action", string("change_page")), ("page", Tag::Int(0))]),
        ),
    ]);
    assert!(matches!(
        Component::from_tag(&tag),
        Err(Error::UnknownVariant { .. })
    ));
}

#[test]
fn hover_show_text_round_trips() {
    agrees_with_vanilla(
        &styled("go", Style {
            hover_event: Some(HoverEvent::ShowText(Box::new(Component::text("tip")))),
            ..Style::new()
        }),
        "0A080004746578740002676F0A000B686F7665725F6576656E74080006616374696F6E000973686F775F7465787408000576616C756500037469700000",
    );
}

#[test]
fn hover_show_entity_writes_the_uuid_as_four_ints() {
    // `UUIDUtil.LENIENT_CODEC` encodes through `UUIDUtil.CODEC`, which is an
    // int stream: the sixteen big-endian bytes read as four big-endian ints,
    // not a dashed string.
    agrees_with_vanilla(
        &styled("e", Style {
            hover_event: Some(HoverEvent::ShowEntity {
                id: Cow::Borrowed("minecraft:pig"),
                uuid: 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF,
                name: None,
            }),
            ..Style::new()
        }),
        "0A080004746578740001650A000B686F7665725F6576656E74080006616374696F6E000B73686F775F656E746974790800026964000D6D696E6563726166743A7069670B0004757569640000000400112233445566778899AABBCCDDEEFF0000",
    );
}

#[test]
fn a_dashed_uuid_is_accepted_on_the_way_in() {
    // The `STRING_CODEC` alternative in `LENIENT_CODEC`. Nothing writes it, so
    // this only has to decode.
    let tag = compound([
        ("text", string("e")),
        (
            "hover_event",
            compound([
                ("action", string("show_entity")),
                ("id", string("minecraft:pig")),
                ("uuid", string("00112233-4455-6677-8899-aabbccddeeff")),
            ]),
        ),
    ]);
    let component = Component::from_tag(&tag).expect("decode");
    let Some(HoverEvent::ShowEntity { uuid, .. }) = component.style.hover_event else {
        panic!("expected a show_entity hover");
    };
    assert_eq!(uuid, 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFF);
}

#[test]
fn hover_show_item_inlines_the_stack_fields() {
    // `ItemStackTemplate.MAP_CODEC` is grouped straight into the event
    // compound, so `id` and `count` sit next to `action` rather than under an
    // `item` key. `count` is absent when it is 1.
    agrees_with_vanilla(
        &styled("i", Style {
            hover_event: Some(HoverEvent::ShowItem {
                id: Cow::Borrowed("minecraft:stone"),
                count: 3,
                components: None,
            }),
            ..Style::new()
        }),
        "0A080004746578740001690A000B686F7665725F6576656E74030005636F756E7400000003080006616374696F6E000973686F775F6974656D0800026964000F6D696E6563726166743A73746F6E650000",
    );
}

// --- the packet-level codec -----------------------------------------------

#[test]
fn a_component_decodes_through_the_codec_traits() {
    let bytes = vanilla("0800026869");
    let mut reader = Reader::new(&bytes);
    let component = Component::decode(&mut reader).expect("decode");
    reader.finish().expect("component fully consumed");
    assert_eq!(component, Component::text("hi"));
}

#[test]
fn an_unknown_field_is_ignored() {
    // A `MapCodec` reads the keys it knows and leaves the rest, so a component
    // from a newer server still decodes.
    let tag = compound([
        ("text", string("hi")),
        ("flarg", string("xy")),
        ("more", Tag::Byte(1)),
    ]);
    assert_eq!(
        Component::from_tag(&tag).expect("decode"),
        Component::text("hi")
    );
}

#[test]
fn a_type_field_is_tolerated_but_never_written() {
    // `StrictEither.decode` switches to the discriminated codec when `type` is
    // present; `StrictEither.encode` always uses the fuzzy one, so nothing
    // vanilla emits ever carries it.
    let tag = compound([("type", string("text")), ("text", string("hi"))]);
    let component = Component::from_tag(&tag).expect("decode");
    assert_eq!(component, Component::text("hi"));

    let mut writer = Writer::new();
    component.encode(&mut writer).expect("encode");
    assert!(matches!(
        decode_tag(writer.as_slice()),
        Tag::Compound(_) | Tag::String(_)
    ));
    assert!(!writer.as_slice().windows(4).any(|window| window == b"type"));
}

#[test]
fn a_verbatim_payload_survives_a_click_event() {
    // `ClickEvent.Custom.payload` is `ExtraCodecs.NBT`, so whatever the sender
    // put there comes back unchanged.
    let mut payload = Compound::new();
    payload.insert("n", Tag::Long(7));
    let component = styled("x", Style {
        click_event: Some(ClickEvent::Custom {
            id: Cow::Borrowed("example:thing"),
            payload: Some(Tag::Compound(payload)),
        }),
        ..Style::new()
    });

    let mut writer = Writer::new();
    component.encode(&mut writer).expect("encode");
    let bytes = writer.into_vec();
    let mut reader = Reader::new(&bytes);
    assert_eq!(Component::decode(&mut reader).expect("decode"), component);
}
