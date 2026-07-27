//! Network NBT tests.
//!
//! Every expected byte string in this file came out of the Minecraft 26.2
//! server jar itself, not out of a specification. A small harness on the
//! classpath of the bundled `server-26.2.jar` built each value with Mojang's
//! own classes, wrote it with `NbtIo.writeAnyTag` — the same call
//! `FriendlyByteBuf.writeNbt` makes — and printed the hex. So an assertion
//! failing here means this crate and the vanilla server disagree, which is the
//! only disagreement worth testing for.

use std::borrow::Cow;

use hyperion_minecraft_proto::{
    Decode, Encode, Error, Reader, Writer,
    nbt::{Compound, List, Tag, decode_optional, encode_optional},
};

/// Parse the hex the vanilla harness printed.
fn vanilla(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "odd-length hex: {hex}");
    (0..hex.len() / 2)
        .map(|index| {
            u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex digit pair")
        })
        .collect()
}

fn encode(tag: &Tag<'_>) -> Vec<u8> {
    let mut writer = Writer::new();
    tag.encode(&mut writer).expect("encode");
    writer.into_vec()
}

/// Assert this crate writes what vanilla wrote, and reads back what it wrote.
fn matches_vanilla(tag: &Tag<'_>, hex: &str) {
    let expected = vanilla(hex);
    assert_eq!(encode(tag), expected, "encoding mismatch for {tag:?}");

    let mut reader = Reader::new(&expected);
    let decoded = Tag::decode(&mut reader).expect("decode");
    reader.finish().expect("tag fully consumed");
    assert_eq!(&decoded, tag, "decoding mismatch");
}

const fn string(value: &str) -> Tag<'_> {
    Tag::String(Cow::Borrowed(value))
}

fn list<'a>(elements: impl IntoIterator<Item = Tag<'a>>) -> Tag<'a> {
    Tag::List(elements.into_iter().collect())
}

fn compound<'a>(entries: impl IntoIterator<Item = (&'a str, Tag<'a>)>) -> Compound<'a> {
    entries
        .into_iter()
        .map(|(name, value)| (Cow::Borrowed(name), value))
        .collect()
}

// --- primitives -----------------------------------------------------------

#[test]
fn scalars_match_vanilla() {
    matches_vanilla(&Tag::Byte(-1), "01FF");
    matches_vanilla(&Tag::Short(0x1234), "021234");
    matches_vanilla(&Tag::Int(0x0102_0304), "0301020304");
    matches_vanilla(&Tag::Long(-2), "04FFFFFFFFFFFFFFFE");
    matches_vanilla(&Tag::Float(1.0), "053F800000");
    matches_vanilla(&Tag::Double(0.5), "063FE0000000000000");
}

#[test]
fn arrays_match_vanilla() {
    matches_vanilla(&Tag::ByteArray(Cow::Borrowed(&[1, 0xFE, 3])), "070000000301FE03");
    matches_vanilla(&Tag::IntArray(vec![1, -2]), "0B0000000200000001FFFFFFFE");
    matches_vanilla(&Tag::LongArray(vec![7]), "0C000000010000000000000007");
}

// --- strings --------------------------------------------------------------

#[test]
fn ascii_string_matches_vanilla() {
    matches_vanilla(&string("hello"), "08000568656C6C6F");
}

#[test]
fn null_is_written_as_two_bytes() {
    // Modified UTF-8's whole point: `StringTag.write` calls
    // `DataOutput.writeUTF`, which spells U+0000 as C0 80 so that no null
    // appears inside the encoding. The prefix counts four bytes for three
    // characters.
    matches_vanilla(&string("a\0b"), "08000461C08062");
}

#[test]
fn supplementary_character_is_written_as_surrogates() {
    // U+1F600 is one four-byte sequence in UTF-8 and two three-byte sequences
    // in modified UTF-8, one per UTF-16 surrogate. A decoder that treats NBT
    // strings as UTF-8 rejects this string outright.
    matches_vanilla(&string("\u{1F600}"), "080006EDA0BDEDB880");
}

#[test]
fn two_byte_sequence_matches_utf8() {
    matches_vanilla(&string("h\u{e9}"), "08000368C3A9");
}

#[test]
fn unpaired_surrogate_is_rejected() {
    // Java hands back a `String` holding a lone surrogate; `str` cannot, so
    // this is the one place the decoder is stricter than the server.
    let bytes = vanilla("080003EDA0BD");
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::InvalidModifiedUtf8));
}

#[test]
fn string_longer_than_the_prefix_is_rejected() {
    let long = "x".repeat(70_000);
    let mut writer = Writer::new();
    assert!(matches!(
        string(&long).encode(&mut writer),
        Err(Error::StringTooLong { max: 65_535, .. })
    ));
}

// --- compounds ------------------------------------------------------------

#[test]
fn root_compound_has_no_name() {
    // The difference between network and file NBT. `NbtIo.writeAnyTag` writes
    // the type byte and goes straight to the payload; `writeUnnamedTag`, which
    // the file format uses, writes an empty name between the two. Two bytes,
    // and getting it wrong desynchronises every packet that carries NBT.
    matches_vanilla(&Tag::Compound(Compound::new()), "0A00");
    matches_vanilla(
        &Tag::Compound(compound([("a", Tag::Int(1))])),
        "0A030001610000000100",
    );
}

#[test]
fn compound_equality_ignores_order() {
    // `CompoundTag` is a `HashMap`, so two encodings differing only in field
    // order are the same value.
    let one = compound([("a", Tag::Int(1)), ("b", Tag::Int(2))]);
    let other = compound([("b", Tag::Int(2)), ("a", Tag::Int(1))]);
    assert_eq!(one, other);
}

#[test]
fn duplicate_keys_keep_the_last() {
    // `loadCompound` does `values.put(key, tag)`, so a repeated key overwrites.
    let bytes = vanilla("0A0300016100000001030001610000000200");
    let mut reader = Reader::new(&bytes);
    let Tag::Compound(decoded) = Tag::decode(&mut reader).expect("decode") else {
        panic!("expected a compound");
    };
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded.get("a"), Some(&Tag::Int(2)));
}

// --- lists ----------------------------------------------------------------

#[test]
fn uniform_list_matches_vanilla() {
    matches_vanilla(
        &list([Tag::Int(1), Tag::Int(2)]),
        "0903000000020000000100000002",
    );
}

#[test]
fn empty_list_declares_tag_end() {
    // `identifyRawElementType` returns 0 when there is nothing to type.
    matches_vanilla(&Tag::List(List::new()), "090000000000");
}

#[test]
fn mixed_list_boxes_every_element() {
    // Since 1.21.5 a list whose elements disagree is written as a list of
    // compounds, each element under the empty key. Anyone still reading the
    // pre-1.21.5 rule that lists are homogeneous will mis-parse this.
    matches_vanilla(
        &list([Tag::Int(1), string("x")]),
        "090A00000002030000000000010008000000017800",
    );
}

#[test]
fn a_compound_that_looks_like_a_box_gets_boxed_too() {
    // `{"": 5}` is indistinguishable from a boxed 5, so `wrapIfNeeded` boxes it
    // again and the far side unboxes exactly once. The neighbouring compound,
    // which is not a box, passes through untouched.
    matches_vanilla(
        &list([
            Tag::Compound(compound([("", Tag::Int(5))])),
            Tag::Compound(compound([("k", Tag::Int(6))])),
        ]),
        "090A000000020A00000300000000000500000300016B0000000600",
    );
}

#[test]
fn nested_list_matches_vanilla() {
    matches_vanilla(
        &list([list([Tag::Byte(1)])]),
        "090900000001010000000101",
    );
}

#[test]
fn a_typed_list_cannot_claim_end_elements() {
    // `loadList` throws "Missing type on ListTag" rather than reading zero-byte
    // elements forever.
    let bytes = vanilla("090000000001");
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::InvalidTagType(0)));
}

// --- absence --------------------------------------------------------------

#[test]
fn a_root_end_tag_means_absent() {
    let mut writer = Writer::new();
    encode_optional(None, &mut writer).expect("encode");
    assert_eq!(writer.as_slice(), [0x00]);

    let bytes = [0x00];
    let mut reader = Reader::new(&bytes);
    assert_eq!(decode_optional(&mut reader).expect("decode"), None);
}

#[test]
fn tag_codec_rejects_a_root_end_tag() {
    // `ByteBufCodecs.tagCodec` turns the null into "Expected non-null compound
    // tag" rather than passing it on.
    let bytes = [0x00];
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::UnexpectedEndTag));
}

// --- hostile input --------------------------------------------------------

#[test]
fn an_array_longer_than_the_input_is_rejected_before_allocating() {
    // The length is attacker-controlled and feeds a reservation. Four bytes of
    // header claiming two billion ints must fail, not allocate 8 GB.
    let bytes = vanilla("0B7FFFFFFF");
    let mut reader = Reader::new(&bytes);
    assert!(matches!(
        Tag::decode(&mut reader),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn a_list_longer_than_the_input_is_rejected_before_allocating() {
    let bytes = vanilla("09037FFFFFFF");
    let mut reader = Reader::new(&bytes);
    assert!(matches!(
        Tag::decode(&mut reader),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn a_negative_array_length_is_rejected() {
    let bytes = vanilla("0BFFFFFFFF");
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::NegativeLength(-1)));
}

#[test]
fn nesting_past_the_limit_is_rejected() {
    // `NbtAccounter.pushDepth` stops at 512 levels. Without the same limit a
    // recursive decoder blows the stack on 25 bytes of input in a loop.
    // One root compound, then 600 compounds each the sole entry of the one
    // above it: type byte, empty name, payload.
    let mut bytes = vec![0x0A];
    for _ in 0..600 {
        bytes.extend_from_slice(&[0x0A, 0x00, 0x00]);
    }
    bytes.resize(bytes.len() + 601, 0x00);
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::NbtTooDeep));
}

#[test]
fn an_unknown_tag_type_is_rejected() {
    let bytes = vanilla("0D");
    let mut reader = Reader::new(&bytes);
    assert_eq!(Tag::decode(&mut reader), Err(Error::InvalidTagType(13)));
}

#[test]
fn a_maximally_nested_tag_survives_a_round_trip() {
    // 512 levels is legal, so encoding and dropping one has to work as well as
    // decoding it. Recursion in any of the three is a crash rather than an
    // error, and the input that reaches the limit is three bytes per level.
    let depth = 512;
    let mut bytes = vec![0x0A];
    for _ in 0..depth - 1 {
        bytes.extend_from_slice(&[0x0A, 0x00, 0x00]);
    }
    bytes.resize(bytes.len() + depth, 0x00);

    let mut reader = Reader::new(&bytes);
    let decoded = Tag::decode(&mut reader).expect("decode at the limit");
    reader.finish().expect("tag fully consumed");
    assert_eq!(encode(&decoded), bytes);
}
