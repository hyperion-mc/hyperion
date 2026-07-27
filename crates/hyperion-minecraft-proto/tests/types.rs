//! The wire value types, and how much of the protocol the generator covers.

use hyperion_minecraft_proto::{
    BlockPos, ChunkPos, Decode, Either, Encode, Error, Holder, Identifier, LengthPrefixed, Reader,
    RegistryId, Uuid, VarInt, Writer,
};

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    assert_eq!(writer.as_slice(), bytes, "encoding mismatch");
    let mut reader = Reader::new(bytes);
    assert_eq!(&T::decode(&mut reader).expect("decode"), value);
    reader.finish().expect("fully consumed");
}

#[test]
fn block_pos_packs_the_way_the_server_does() {
    // `BlockPos.asLong` puts x in the top 26 bits, z in the next 26 and y in
    // the low 12: (1 << 38) | (3 << 12) | 2.
    let packed = (1i64 << 38) | (3i64 << 12) | 2;
    assert_eq!(BlockPos::new(1, 2, 3).to_bits(), packed);
    round_trip(&BlockPos::new(1, 2, 3), &packed.to_be_bytes());
}

#[test]
fn block_pos_sign_extends_each_field_from_its_own_width() {
    // Each coordinate is two's-complement within 26 or 12 bits, so a negative
    // one only decodes correctly if the sign is extended from that width.
    for pos in [
        BlockPos::new(-1, -1, -1),
        BlockPos::new(-30_000_000, -2048, 30_000_000),
        BlockPos::new(0, 2047, 0),
    ] {
        assert_eq!(BlockPos::from_bits(pos.to_bits()), pos, "{pos:?}");
    }
}

#[test]
fn chunk_pos_puts_z_in_the_high_half() {
    let packed = (2i64 << 32) | 0xFFFF_FFFF;
    assert_eq!(ChunkPos::new(-1, 2).to_bits(), packed);
    assert_eq!(ChunkPos::from_bits(packed), ChunkPos::new(-1, 2));
}

#[test]
fn an_identifier_defaults_its_namespace() {
    let bare = Identifier::new("stone").expect("valid");
    assert_eq!(bare.namespace(), "minecraft");
    assert_eq!(bare.path(), "stone");

    let full = Identifier::new("hyperion:bed_wars").expect("valid");
    assert_eq!(full.namespace(), "hyperion");
    assert_eq!(full.path(), "bed_wars");
}

#[test]
fn an_identifier_rejects_characters_the_server_rejects() {
    // `Identifier.tryParse` allows a slash in the path but not in the
    // namespace, and no upper case anywhere.
    assert!(Identifier::new("minecraft:block/stone").is_ok());
    assert_eq!(
        Identifier::new("Minecraft:stone"),
        Err(Error::InvalidIdentifier("Minecraft:stone".to_owned()))
    );
    assert!(Identifier::new("a/b:stone").is_err());
    assert!(Identifier::new("minecraft:").is_err());
}

#[test]
fn a_uuid_displays_in_the_canonical_form() {
    assert_eq!(
        Uuid(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF).to_string(),
        "01234567-89ab-cdef-0123-456789abcdef"
    );
}

#[test]
fn a_holder_is_an_id_plus_one_or_zero_and_the_value() {
    round_trip(&Holder::<VarInt>::Reference(RegistryId(0)), &[0x01]);
    round_trip(&Holder::Inline(VarInt(7)), &[0x00, 0x07]);
}

#[test]
fn either_writes_true_for_its_left() {
    round_trip(&Either::<VarInt, bool>::Left(VarInt(1)), &[0x01, 0x01]);
    round_trip(&Either::<VarInt, bool>::Right(false), &[0x00, 0x00]);
}

#[test]
fn a_length_prefix_must_match_what_the_value_consumes() {
    round_trip(&LengthPrefixed(VarInt(300)), &[0x02, 0xAC, 0x02]);

    // A prefix promising three bytes where the value uses two is a layout
    // disagreement, not something to read past.
    let mut reader = Reader::new(&[0x03, 0xAC, 0x02, 0x00]);
    assert_eq!(
        LengthPrefixed::<VarInt>::decode(&mut reader),
        Err(Error::TrailingBytes(1))
    );
}

/// How many packets `build.rs` wrote, and how many it declined.
///
/// Pinned so that a change which silently stops generating a packet fails
/// here rather than being noticed when a codec goes missing.
const COVERAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/coverage.txt"));

#[test]
fn the_generator_covers_every_layout_it_can() {
    let counts: Vec<usize> = COVERAGE
        .split_whitespace()
        .map(|field| field.parse().expect("coverage counts are numbers"))
        .collect();
    let [written, declined] = counts[..] else {
        panic!("coverage.txt should hold two counts, found {COVERAGE:?}");
    };
    assert_eq!(written, 176, "packet classes generated");
    assert_eq!(declined, 3, "layouts the generator declined");
}
