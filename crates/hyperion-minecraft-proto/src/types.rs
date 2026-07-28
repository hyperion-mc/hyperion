//! The value types a packet field can have.
//!
//! Each one exists because the wire distinguishes something Rust's primitives
//! do not: a [`BlockPos`] and an `i64` are the same eight bytes and mean
//! different things.
//!
//! How many bytes a *number* costs is not one of those distinctions. That is
//! the wire's choice about a value Rust already has a type for, so it lives in
//! `#[proto(varint)]` on the field rather than in a wrapper the caller has to
//! spell; see [`crate::Encode`]'s derive. [`VarInt`] and [`VarLong`] remain
//! for the positions an attribute cannot reach, such as inside an [`Either`].

use std::fmt;

use crate::{Decode, Encode, Error, Reader, Result, Writer, codec};

/// An `i32` that carries its own seven-bits-per-byte encoding.
///
/// A packet field says this with `#[proto(varint)]` and stays an `i32`. This
/// type is for the positions where no attribute reaches the value: a type
/// parameter of [`Either`], [`Holder`] or [`LengthPrefixed`], and the codec
/// layer's own plumbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarInt(pub i32);

/// An `i64` that carries its own seven-bits-per-byte encoding, as [`VarInt`]
/// does for `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarLong(pub i64);

/// A UUID, written as two big-endian 64-bit halves, most significant first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Uuid(pub u128);

/// An index into a synced registry, written as a `VarInt`.
///
/// Which registry it indexes is a property of the field, not of the value, so
/// it lives in the field's documentation rather than in the type. Resolve one
/// against the tables in [`crate::generated::registry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RegistryId(pub i32);

macro_rules! var_codec {
    ($name:ident, $inner:ty, $write:ident, $read:ident) => {
        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl Encode for $name {
            fn encode(&self, writer: &mut Writer) -> Result<()> {
                writer.$write(self.0);
                Ok(())
            }
        }

        impl Decode<'_> for $name {
            fn decode(reader: &mut Reader<'_>) -> Result<Self> {
                reader.$read().map(Self)
            }
        }
    };
}

var_codec!(VarInt, i32, var_int, var_int);
var_codec!(VarLong, i64, var_long, var_long);
var_codec!(RegistryId, i32, var_int, var_int);

impl From<u128> for Uuid {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<Uuid> for u128 {
    fn from(value: Uuid) -> Self {
        value.0
    }
}

impl fmt::Display for Uuid {
    /// The canonical 8-4-4-4-12 hyphenated form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = format!("{:032x}", self.0);
        write!(
            f,
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }
}

impl Encode for Uuid {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.uuid(self.0);
        Ok(())
    }
}

impl Decode<'_> for Uuid {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        reader.uuid().map(Self)
    }
}

/// A block state's network id: an index into the global state registry.
///
/// A newtype rather than a bare `u32` because that is what the wire carries
/// for a particle's block, for a block update and for a palette entry, and
/// none of those is interchangeable with the count or the id sitting next to
/// it. [`crate::block_state`] is what turns a block name and its properties
/// into one of these.
///
/// The numbering is dense but arbitrary and moves with almost every game
/// version, so an id is only meaningful against the version that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockStateId(u32);

impl BlockStateId {
    /// The state of that id, if this version has one.
    ///
    /// # Errors
    /// [`Error::InvalidBlockState`] for a negative id or one at or past
    /// [`crate::block_state::STATE_COUNT`]. Both are only reachable from the
    /// wire; a state named in source goes through [`crate::block_state`].
    pub const fn from_raw(raw: i32) -> Result<Self> {
        // `u32::try_from` is not const, and a bad id here is a decode error
        // rather than a panic, so the bounds are written out.
        if raw < 0 || raw.unsigned_abs() >= crate::block_state::STATE_COUNT {
            return Err(Error::InvalidBlockState { value: raw });
        }
        Ok(Self(raw.unsigned_abs()))
    }

    /// A state id this crate computed, such as one from [`crate::block_state`].
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// The id as the wire carries it.
    ///
    /// # Panics
    /// Never in practice: every state id is below
    /// [`crate::block_state::STATE_COUNT`], which is five orders of magnitude
    /// under `i32::MAX`. Written as a checked conversion rather than a cast so
    /// that a value which somehow got past [`Self::new`] fails here instead of
    /// wrapping negative and encoding as a five-byte `VarInt`.
    #[must_use]
    pub fn to_raw(self) -> i32 {
        i32::try_from(self.0).expect("a block state id is far below i32::MAX")
    }

    /// The id as an index.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A block position, packed into one 64-bit word.
///
/// `BlockPos.asLong` gives x the top 26 bits, z the next 26 and y the low 12,
/// each two's-complement within its own width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BlockPos {
    /// East-west coordinate, 26 bits.
    pub x: i32,
    /// Vertical coordinate, 12 bits.
    pub y: i32,
    /// North-south coordinate, 26 bits.
    pub z: i32,
}

impl BlockPos {
    const HORIZONTAL_BITS: u32 = 26;
    const VERTICAL_BITS: u32 = 64 - 2 * Self::HORIZONTAL_BITS;
    const X_SHIFT: u32 = Self::VERTICAL_BITS + Self::HORIZONTAL_BITS;
    const Z_SHIFT: u32 = Self::VERTICAL_BITS;

    /// A position at the given coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The packed form written to the wire.
    #[must_use]
    pub const fn to_bits(self) -> i64 {
        let x = (self.x as i64 & mask(Self::HORIZONTAL_BITS)) << Self::X_SHIFT;
        let z = (self.z as i64 & mask(Self::HORIZONTAL_BITS)) << Self::Z_SHIFT;
        let y = self.y as i64 & mask(Self::VERTICAL_BITS);
        x | z | y
    }

    /// Unpack the wire form, sign-extending each field from its own width.
    #[must_use]
    pub const fn from_bits(bits: i64) -> Self {
        Self {
            x: sign_extend(bits >> Self::X_SHIFT, Self::HORIZONTAL_BITS),
            y: sign_extend(bits, Self::VERTICAL_BITS),
            z: sign_extend(bits >> Self::Z_SHIFT, Self::HORIZONTAL_BITS),
        }
    }
}

/// A chunk position, packed into one 64-bit word: z high, x low.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ChunkPos {
    /// East-west chunk coordinate.
    pub x: i32,
    /// North-south chunk coordinate.
    pub z: i32,
}

impl ChunkPos {
    /// A position at the given chunk coordinates.
    #[must_use]
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// The packed form written to the wire.
    #[must_use]
    pub const fn to_bits(self) -> i64 {
        ((self.z as i64) << 32) | (self.x as i64 & 0xFFFF_FFFF)
    }

    /// Unpack the wire form.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "taking the low half is what unpacking a packed pair means"
    )]
    pub const fn from_bits(bits: i64) -> Self {
        Self {
            x: bits as i32,
            z: (bits >> 32) as i32,
        }
    }
}

const fn mask(bits: u32) -> i64 {
    (1i64 << bits) - 1
}

/// Sign-extend the low `bits` of `value` as a two's-complement integer.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the shifts leave at most `bits` significant bits, and `bits` is under 32"
)]
const fn sign_extend(value: i64, bits: u32) -> i32 {
    (value << (64 - bits) >> (64 - bits)) as i32
}

impl Encode for BlockPos {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.to_bits());
        Ok(())
    }
}

impl Decode<'_> for BlockPos {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        reader.i64().map(Self::from_bits)
    }
}

impl Encode for ChunkPos {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.to_bits());
        Ok(())
    }
}

impl Decode<'_> for ChunkPos {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        reader.i64().map(Self::from_bits)
    }
}

/// A namespaced name such as `minecraft:stone`, borrowed from the buffer.
///
/// `Identifier.parse` accepts an unqualified name and fills in the
/// `minecraft` namespace, so [`Identifier::namespace`] answers `minecraft`
/// for a value written without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier<'a>(&'a str);

impl<'a> Identifier<'a> {
    /// The namespace assumed when a name is written without one.
    pub const DEFAULT_NAMESPACE: &'static str = "minecraft";

    /// Validate a namespaced name, matching `Identifier.tryParse`.
    ///
    /// # Errors
    /// Returns [`Error::InvalidIdentifier`] when a character is outside the
    /// permitted set for its half of the name.
    pub fn new(text: &'a str) -> Result<Self> {
        let (namespace, path) = text.split_once(':').unwrap_or(("", text));
        let valid = namespace.bytes().all(is_namespace_byte)
            && !path.is_empty()
            && path.bytes().all(is_path_byte);
        if valid {
            Ok(Self(text))
        } else {
            Err(Error::InvalidIdentifier(text.to_owned()))
        }
    }

    /// The whole name as written, without a namespace if none was written.
    #[must_use]
    pub const fn as_str(&self) -> &'a str {
        self.0
    }

    /// The namespace, defaulting to `minecraft`.
    #[must_use]
    pub fn namespace(&self) -> &'a str {
        self.0
            .split_once(':')
            .map_or(Self::DEFAULT_NAMESPACE, |(namespace, _)| namespace)
    }

    /// The part after the colon, or the whole name if there is no colon.
    #[must_use]
    pub fn path(&self) -> &'a str {
        self.0.split_once(':').map_or(self.0, |(_, path)| path)
    }
}

const fn is_namespace_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-')
}

const fn is_path_byte(byte: u8) -> bool {
    is_namespace_byte(byte) || byte == b'/'
}

impl fmt::Display for Identifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Encode for Identifier<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.0)
    }
}

impl<'a> Decode<'a> for Identifier<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Self::new(reader.string()?)
    }
}

/// Which face of a block a ray struck.
///
/// Discriminants are `Direction`'s declaration order, which is what
/// `FriendlyByteBuf.writeEnum` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum Direction {
    /// Negative y.
    Down = 0,
    /// Positive y.
    Up = 1,
    /// Negative z.
    North = 2,
    /// Positive z.
    South = 3,
    /// Negative x.
    West = 4,
    /// Positive x.
    East = 5,
}

/// Where a player's ray met a block.
///
/// The offsets are the hit point relative to the block's own corner, which is
/// how `FriendlyByteBuf.writeBlockHitResult` sends it: the absolute position
/// is recovered by adding them to `block_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Encode, Decode)]
pub struct BlockHitResult {
    /// The block that was hit.
    pub block_pos: BlockPos,
    /// The face that was hit.
    pub direction: Direction,
    /// Hit point's x offset within the block.
    pub offset_x: f32,
    /// Hit point's y offset within the block.
    pub offset_y: f32,
    /// Hit point's z offset within the block.
    pub offset_z: f32,
    /// Whether the ray started inside the block.
    pub inside: bool,
    /// Whether the ray was stopped by the world border rather than the block.
    pub world_border_hit: bool,
}

/// A value that is either a registry reference or written out in full.
///
/// A `VarInt` of zero introduces the inline form; anything else is a registry
/// id plus one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Holder<T> {
    /// An id in the registry the field names.
    Reference(RegistryId),
    /// The value itself, for entries the client does not have.
    Inline(T),
}

impl<T: Encode> Encode for Holder<T> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        match self {
            Self::Reference(id) => {
                let raw = id.0.checked_add(1).ok_or(Error::NegativeLength(id.0))?;
                writer.var_int(raw);
                Ok(())
            }
            Self::Inline(value) => {
                writer.var_int(0);
                value.encode(writer)
            }
        }
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for Holder<T> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        match reader.var_int()? {
            0 => Ok(Self::Inline(T::decode(reader)?)),
            raw => Ok(Self::Reference(RegistryId(raw - 1))),
        }
    }
}

/// One of two layouts, selected by a leading boolean.
///
/// `true` selects [`Either::Left`], matching `FriendlyByteBuf.writeEither`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Either<L, R> {
    /// The layout the `true` discriminant selects.
    Left(L),
    /// The layout the `false` discriminant selects.
    Right(R),
}

impl<L: Encode, R: Encode> Encode for Either<L, R> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        match self {
            Self::Left(value) => {
                writer.bool(true);
                value.encode(writer)
            }
            Self::Right(value) => {
                writer.bool(false);
                value.encode(writer)
            }
        }
    }
}

impl<'a, L: Decode<'a>, R: Decode<'a>> Decode<'a> for Either<L, R> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        if reader.bool()? {
            Ok(Self::Left(L::decode(reader)?))
        } else {
            Ok(Self::Right(R::decode(reader)?))
        }
    }
}

/// A value behind a `VarInt` byte count.
///
/// The count lets a reader skip a value it cannot parse, so decoding checks
/// that the inner value consumed exactly the bytes the count promised rather
/// than trusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LengthPrefixed<T>(pub T);

impl<T: Encode> Encode for LengthPrefixed<T> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let mut inner = Writer::new();
        self.0.encode(&mut inner)?;
        writer.byte_array(inner.as_slice())
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for LengthPrefixed<T> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let bytes = reader.byte_array()?;
        let mut inner = Reader::new(bytes);
        let value = T::decode(&mut inner)?;
        inner.finish()?;
        Ok(Self(value))
    }
}

// ---------------------------------------------------------------------------
// Primitives and containers
// ---------------------------------------------------------------------------

macro_rules! fixed_codec {
    ($($ty:ty => $write:ident / $read:ident),* $(,)?) => {
        $(
            impl Encode for $ty {
                fn encode(&self, writer: &mut Writer) -> Result<()> {
                    writer.$write(*self);
                    Ok(())
                }
            }

            impl Decode<'_> for $ty {
                fn decode(reader: &mut Reader<'_>) -> Result<Self> {
                    reader.$read()
                }
            }
        )*
    };
}

fixed_codec! {
    bool => bool / bool,
    u8 => u8 / u8,
    i8 => i8 / i8,
    u16 => u16 / u16,
    i16 => i16 / i16,
    i32 => i32 / i32,
    i64 => i64 / i64,
    f32 => f32 / f32,
    f64 => f64 / f64,
}

/// A packet whose body is empty.
impl Encode for () {
    fn encode(&self, _writer: &mut Writer) -> Result<()> {
        Ok(())
    }
}

impl Decode<'_> for () {
    fn decode(_reader: &mut Reader<'_>) -> Result<Self> {
        Ok(())
    }
}

impl Encode for &str {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self)
    }
}

impl<'a> Decode<'a> for &'a str {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        reader.string()
    }
}

impl Encode for &[u8] {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.byte_array(self)
    }
}

impl<'a> Decode<'a> for &'a [u8] {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        reader.byte_array()
    }
}

/// A boolean, then the value when it is present.
impl<T: Encode> Encode for Option<T> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        match self {
            Some(value) => {
                writer.bool(true);
                value.encode(writer)
            }
            None => {
                writer.bool(false);
                Ok(())
            }
        }
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for Option<T> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        if reader.bool()? {
            Ok(Some(T::decode(reader)?))
        } else {
            Ok(None)
        }
    }
}

/// A `VarInt` count, then that many elements.
impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        codec::write_count(writer, self.len(), None)?;
        for item in self {
            item.encode(writer)?;
        }
        Ok(())
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for Vec<T> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let count = codec::read_count(reader, None)?;
        // Capacity is clamped to the bytes actually present: the count comes
        // off the wire and every element costs at least one byte, so a huge
        // count on a short buffer must not reserve for it.
        let mut items = Self::with_capacity(count.min(reader.remaining_len()));
        for _ in 0..count {
            items.push(T::decode(reader)?);
        }
        Ok(items)
    }
}

/// A key and a value back to back, which is how a map entry is written.
impl<K: Encode, V: Encode> Encode for (K, V) {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.0.encode(writer)?;
        self.1.encode(writer)
    }
}

impl<'a, K: Decode<'a>, V: Decode<'a>> Decode<'a> for (K, V) {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok((K::decode(reader)?, V::decode(reader)?))
    }
}

// The domain types the extractor recovered from the server's own codecs. They
// live beside the hand-written primitives because a reader looking for `Vec3`
// does not care which side of that line it fell on; the private module exists
// only to give the generated file an import scope of its own.
mod jar {
    include!(concat!(env!("OUT_DIR"), "/types.rs"));
}

pub use jar::*;
