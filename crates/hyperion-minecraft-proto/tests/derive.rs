//! What `#[derive(Encode, Decode)]` produces.
//!
//! These are the shapes the generator emits, exercised on hand-written types
//! so a failure points at the derive rather than at a packet layout.

use hyperion_minecraft_proto::{Decode, Encode, Error, Identifier, Reader, Result, Uuid, Writer};

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), bytes, "encoding mismatch");

    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Unit;

#[test]
fn a_unit_struct_occupies_no_bytes() {
    round_trip(&Unit, &[]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Newtype(#[proto(varint)] i32);

#[test]
fn a_newtype_is_its_field() {
    round_trip(&Newtype(300), &[0xAC, 0x02]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Encodings {
    plain: i32,
    #[proto(varint)]
    short: i32,
    #[proto(varlong)]
    long: i64,
    #[proto(varint)]
    maybe: Option<i32>,
}

#[test]
fn an_encoding_attribute_selects_the_wire_form_of_a_plain_integer() {
    // The same 300 four ways: four fixed bytes, two `VarInt` bytes, two
    // `VarLong` bytes, and the same again behind a present-flag.
    round_trip(
        &Encodings {
            plain: 300,
            short: 300,
            long: 300,
            maybe: Some(300),
        },
        &[
            0x00, 0x00, 0x01, 0x2C, // plain
            0xAC, 0x02, // short
            0xAC, 0x02, // long
            0x01, 0xAC, 0x02, // maybe
        ],
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Fields<'a> {
    #[proto(varint)]
    count: i32,
    flag: bool,
    name: &'a str,
    id: Uuid,
}

#[test]
fn fields_are_written_in_declaration_order() {
    round_trip(
        &Fields {
            count: 1,
            flag: true,
            name: "hi",
            id: Uuid(0),
        },
        &[
            0x01, 0x01, 0x02, b'h', b'i', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct Bounded<'a> {
    #[proto(max_len = 4)]
    short: &'a str,
    #[proto(varint, max_count = 2)]
    few: Vec<i32>,
    #[proto(max_len = 3)]
    maybe: Option<&'a str>,
    #[proto(max_len = 2)]
    pages: Vec<&'a str>,
}

#[test]
fn limits_reach_the_type_they_constrain() {
    round_trip(
        &Bounded {
            short: "abcd",
            few: vec![7],
            maybe: Some("xy"),
            pages: vec!["a", "b"],
        },
        &[
            0x04, b'a', b'b', b'c', b'd', // short
            0x01, 0x07, // few
            0x01, 0x02, b'x', b'y', // maybe
            0x02, 0x01, b'a', 0x01, b'b', // pages, one limit per page
        ],
    );
}

#[test]
fn an_over_long_string_is_refused_on_write() {
    let value = Bounded {
        short: "abcde",
        few: Vec::new(),
        maybe: None,
        pages: Vec::new(),
    };
    let mut writer = Writer::new();
    assert_eq!(
        value.encode(&mut writer),
        Err(Error::StringTooLong { length: 5, max: 4 })
    );
}

#[test]
fn an_over_long_list_is_refused_on_read() {
    // Three elements where the field permits two: the server would throw, so
    // decoding one must not quietly succeed.
    let bytes = [0x00, 0x03, 0x01, 0x02, 0x03, 0x00, 0x00];
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        Bounded::decode(&mut reader),
        Err(Error::ListTooLong { length: 3, max: 2 })
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[repr(i32)]
enum Intent {
    Status = 1,
    Login = 2,
    Transfer = 3,
}

#[test]
fn an_enum_is_a_varint_discriminant() {
    round_trip(&Intent::Status, &[0x01]);
    round_trip(&Intent::Transfer, &[0x03]);
}

#[test]
fn an_unknown_discriminant_is_an_error_not_a_variant() {
    let mut reader = Reader::new(&[0x00]);
    assert_eq!(
        Intent::decode(&mut reader),
        Err(Error::InvalidEnum {
            name: "Intent",
            value: 0
        })
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum Implicit {
    First,
    Second,
    Tenth = 10,
    Eleventh,
}

#[test]
fn implicit_discriminants_count_the_way_rust_does() {
    round_trip(&Implicit::First, &[0x00]);
    round_trip(&Implicit::Second, &[0x01]);
    round_trip(&Implicit::Tenth, &[0x0A]);
    round_trip(&Implicit::Eleventh, &[0x0B]);
}

/// A codec that is not the derive's default shape, reached with `with`.
///
/// The signatures are the contract the derive calls: a shared reference in
/// and a `Result` out, whatever the field's own type would allow.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::unnecessary_wraps,
    reason = "the shape is fixed by what `#[proto(with = ...)]` calls"
)]
mod doubled {
    use super::{Reader, Result, Writer};

    pub fn encode(value: &i32, writer: &mut Writer) -> Result<()> {
        writer.var_int(value * 2);
        Ok(())
    }

    pub fn decode(reader: &mut Reader<'_>) -> Result<i32> {
        Ok(reader.var_int()? / 2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct Custom {
    #[proto(with = doubled)]
    value: i32,
}

#[test]
fn with_replaces_the_whole_field_codec() {
    round_trip(&Custom { value: 21 }, &[0x2A]);
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct Nested<'a> {
    #[proto(varint)]
    outer: i32,
    inner: Fields<'a>,
    list: Vec<Fields<'a>>,
    id: Option<Identifier<'a>>,
}

#[test]
fn nested_structs_delegate_to_their_own_codecs() {
    let inner = Fields {
        count: 2,
        flag: false,
        name: "a",
        id: Uuid(1),
    };
    let mut expected = vec![0x09];
    let mut one = Writer::new();
    inner.encode(&mut one).expect("encode inner");
    expected.extend_from_slice(one.as_slice());
    expected.push(0x01);
    expected.extend_from_slice(one.as_slice());
    expected.push(0x00);
    round_trip(
        &Nested {
            outer: 9,
            inner,
            list: vec![inner],
            id: None,
        },
        &expected,
    );
}
