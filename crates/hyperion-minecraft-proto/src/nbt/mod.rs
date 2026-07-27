//! Network NBT.
//!
//! Network NBT is not file NBT. `FriendlyByteBuf.writeNbt` goes through
//! `NbtIo.writeAnyTag`, which writes a type byte and then the payload; the
//! named-root form lives in `NbtIo.writeUnnamedTag`, which the network never
//! calls. So the root carries neither a name nor the empty-name length prefix
//! that a file root still has.
//!
//! A root `TAG_End` stands for absence: `FriendlyByteBuf.readNbt` maps it to
//! `null`, which is why [`decode_optional`] exists alongside the [`Decode`]
//! impl. `ByteBufCodecs.tagCodec` rejects that case, and so does [`Tag`]'s own
//! impl.
//!
//! What is deliberately not reproduced is `NbtAccounter`'s byte quota. Those
//! numbers (48 bytes for a compound, 36 for a map entry) size Java heap
//! objects, not wire bytes, and they exist because Java allocates before it
//! knows the input is short. A [`Reader`] is already backed by the whole
//! message, so the check that matters here is that a declared length is
//! actually present, which every array and list read does before reserving.
//! The 512-deep nesting limit is real and is enforced.

pub(crate) mod mutf8;

use std::borrow::Cow;

use crate::{Decode, Encode, Error, Reader, Result, Writer};

/// Deepest nesting a tag may reach (`Tag.MAX_DEPTH`, `NbtAccounter.MAX_STACK_DEPTH`).
pub const MAX_DEPTH: usize = 512;

/// `TAG_End`, which on the wire means "no value".
pub const TAG_END: u8 = 0;
/// `TAG_Byte`.
pub const TAG_BYTE: u8 = 1;
/// `TAG_Short`.
pub const TAG_SHORT: u8 = 2;
/// `TAG_Int`.
pub const TAG_INT: u8 = 3;
/// `TAG_Long`.
pub const TAG_LONG: u8 = 4;
/// `TAG_Float`.
pub const TAG_FLOAT: u8 = 5;
/// `TAG_Double`.
pub const TAG_DOUBLE: u8 = 6;
/// `TAG_Byte_Array`.
pub const TAG_BYTE_ARRAY: u8 = 7;
/// `TAG_String`.
pub const TAG_STRING: u8 = 8;
/// `TAG_List`.
pub const TAG_LIST: u8 = 9;
/// `TAG_Compound`.
pub const TAG_COMPOUND: u8 = 10;
/// `TAG_Int_Array`.
pub const TAG_INT_ARRAY: u8 = 11;
/// `TAG_Long_Array`.
pub const TAG_LONG_ARRAY: u8 = 12;

/// One NBT value.
///
/// There is no `End` variant: `TAG_End` is a terminator and an absence marker,
/// never a value a field can hold. Strings and byte arrays borrow the input
/// where they can, so decoding a typical compound allocates only for its own
/// entry list.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag<'a> {
    /// A signed byte, which is also how NBT spells a boolean.
    Byte(i8),
    /// A signed short.
    Short(i16),
    /// A signed int.
    Int(i32),
    /// A signed long.
    Long(i64),
    /// A single-precision float.
    Float(f32),
    /// A double-precision float.
    Double(f64),
    /// A length-prefixed run of bytes.
    ByteArray(Cow<'a, [u8]>),
    /// A modified-UTF-8 string.
    String(Cow<'a, str>),
    /// A list, which since 1.21.5 may hold mixed types. See [`List`].
    List(List<'a>),
    /// A map of names to tags.
    Compound(Compound<'a>),
    /// A length-prefixed run of big-endian ints.
    IntArray(Vec<i32>),
    /// A length-prefixed run of big-endian longs.
    LongArray(Vec<i64>),
}

impl<'a> Tag<'a> {
    /// The type byte that introduces this tag.
    #[must_use]
    pub const fn id(&self) -> u8 {
        match self {
            Self::Byte(_) => TAG_BYTE,
            Self::Short(_) => TAG_SHORT,
            Self::Int(_) => TAG_INT,
            Self::Long(_) => TAG_LONG,
            Self::Float(_) => TAG_FLOAT,
            Self::Double(_) => TAG_DOUBLE,
            Self::ByteArray(_) => TAG_BYTE_ARRAY,
            Self::String(_) => TAG_STRING,
            Self::List(_) => TAG_LIST,
            Self::Compound(_) => TAG_COMPOUND,
            Self::IntArray(_) => TAG_INT_ARRAY,
            Self::LongArray(_) => TAG_LONG_ARRAY,
        }
    }

    /// The string this tag holds, if it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// The compound this tag holds, if it is one.
    #[must_use]
    pub const fn as_compound(&self) -> Option<&Compound<'a>> {
        match self {
            Self::Compound(value) => Some(value),
            _ => None,
        }
    }

    /// The list this tag holds, if it is one.
    #[must_use]
    pub const fn as_list(&self) -> Option<&List<'a>> {
        match self {
            Self::List(value) => Some(value),
            _ => None,
        }
    }

    /// This tag read as a boolean, which NBT stores as a byte.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Byte(value) => Some(*value != 0),
            _ => None,
        }
    }

    /// This tag widened to an `i64`, for any of the integral types.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Byte(value) => Some(i64::from(*value)),
            Self::Short(value) => Some(i64::from(*value)),
            Self::Int(value) => Some(i64::from(*value)),
            Self::Long(value) => Some(*value),
            _ => None,
        }
    }

    fn write_payload(&self, writer: &mut Writer, depth: usize) -> Result<()> {
        match self {
            Self::Byte(value) => writer.i8(*value),
            Self::Short(value) => writer.i16(*value),
            Self::Int(value) => writer.i32(*value),
            Self::Long(value) => writer.i64(*value),
            Self::Float(value) => writer.f32(*value),
            Self::Double(value) => writer.f64(*value),
            Self::ByteArray(value) => {
                writer.i32(length_prefix(value.len())?);
                writer.raw(value);
            }
            Self::String(value) => mutf8::write(writer, value)?,
            Self::List(value) => value.write_payload(writer, depth)?,
            Self::Compound(value) => value.write_payload(writer, depth)?,
            Self::IntArray(value) => {
                writer.i32(length_prefix(value.len())?);
                for element in value {
                    writer.i32(*element);
                }
            }
            Self::LongArray(value) => {
                writer.i32(length_prefix(value.len())?);
                for element in value {
                    writer.i64(*element);
                }
            }
        }
        Ok(())
    }

    /// Read a tag that cannot contain another one.
    ///
    /// Containers are handled by [`read_nested`], which is where the whole
    /// depth question lives; keeping them out of here is what lets that
    /// function be a loop.
    fn read_leaf(reader: &mut Reader<'a>, id: u8) -> Result<Self> {
        Ok(match id {
            TAG_BYTE => Self::Byte(reader.i8()?),
            TAG_SHORT => Self::Short(reader.i16()?),
            TAG_INT => Self::Int(reader.i32()?),
            TAG_LONG => Self::Long(reader.i64()?),
            TAG_FLOAT => Self::Float(reader.f32()?),
            TAG_DOUBLE => Self::Double(reader.f64()?),
            TAG_BYTE_ARRAY => {
                let length = counted(reader, 1)?;
                Self::ByteArray(Cow::Borrowed(reader.take(length)?))
            }
            TAG_STRING => Self::String(mutf8::read(reader)?),
            TAG_INT_ARRAY => {
                let length = counted(reader, 4)?;
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(reader.i32()?);
                }
                Self::IntArray(values)
            }
            TAG_LONG_ARRAY => {
                let length = counted(reader, 8)?;
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    values.push(reader.i64()?);
                }
                Self::LongArray(values)
            }
            other => return Err(Error::InvalidTagType(other)),
        })
    }
}

impl Encode for Tag<'_> {
    /// Write the tag as `NbtIo.writeAnyTag` does: type byte, then payload.
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(self.id());
        self.write_payload(writer, 0)
    }
}

impl<'a> Decode<'a> for Tag<'a> {
    /// Read one tag, rejecting a root `TAG_End`.
    ///
    /// This is `ByteBufCodecs.tagCodec`, which turns the `null` that
    /// `FriendlyByteBuf.readNbt` would return into a decoder error.
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        decode_optional(reader)?.ok_or(Error::UnexpectedEndTag)
    }
}

/// Write an optional tag, spelling `None` as a bare `TAG_End`.
///
/// This is `FriendlyByteBuf.writeNbt`, which substitutes `EndTag.INSTANCE` for
/// a null tag, and `ByteBufCodecs.optionalTagCodec`.
///
/// # Errors
/// Propagates whatever the tag's own encoding fails on.
pub fn encode_optional(tag: Option<&Tag<'_>>, writer: &mut Writer) -> Result<()> {
    match tag {
        Some(tag) => tag.encode(writer),
        None => {
            writer.u8(TAG_END);
            Ok(())
        }
    }
}

/// Read an optional tag, reading a bare `TAG_End` as `None`.
///
/// # Errors
/// Returns an error on a malformed tag or truncated input.
pub fn decode_optional<'a>(reader: &mut Reader<'a>) -> Result<Option<Tag<'a>>> {
    let id = reader.u8()?;
    if id == TAG_END {
        return Ok(None);
    }
    read_nested(reader, id).map(Some)
}

/// A map of names to tags.
///
/// Entries keep insertion order. The server's `CompoundTag` is a `HashMap` and
/// so has no order at all, which means byte-for-byte agreement with a vanilla
/// encoder is not a property either side can offer; keeping order at least
/// makes this crate's own output reproducible. Equality ignores order, matching
/// Java's map equality.
#[derive(Debug, Clone, Default)]
pub struct Compound<'a> {
    entries: Vec<(Cow<'a, str>, Tag<'a>)>,
}

impl<'a> Compound<'a> {
    /// An empty compound.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when there are no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The tag stored under `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Tag<'a>> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// Store `value` under `name`, replacing and returning any previous tag.
    pub fn insert(&mut self, name: impl Into<Cow<'a, str>>, value: Tag<'a>) -> Option<Tag<'a>> {
        let name = name.into();
        match self.entries.iter_mut().find(|(key, _)| *key == name) {
            Some(entry) => Some(std::mem::replace(&mut entry.1, value)),
            None => {
                self.entries.push((name, value));
                None
            }
        }
    }

    /// Store `value` under `name` when `value` is present, and do nothing when
    /// it is not.
    ///
    /// Every optional field in a component codec behaves this way: DFU's
    /// `optionalFieldOf` omits the key rather than writing a null.
    pub fn insert_optional(&mut self, name: &'static str, value: Option<Tag<'a>>) {
        if let Some(value) = value {
            self.insert(name, value);
        }
    }

    /// Remove and return the tag stored under `name`.
    pub fn remove(&mut self, name: &str) -> Option<Tag<'a>> {
        let index = self.entries.iter().position(|(key, _)| key == name)?;
        Some(self.entries.remove(index).1)
    }

    /// The entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Tag<'a>)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_ref(), value))
    }

    fn write_payload(&self, writer: &mut Writer, depth: usize) -> Result<()> {
        if depth >= MAX_DEPTH {
            return Err(Error::NbtTooDeep);
        }
        for (name, value) in &self.entries {
            writer.u8(value.id());
            mutf8::write(writer, name)?;
            value.write_payload(writer, depth + 1)?;
        }
        writer.u8(TAG_END);
        Ok(())
    }
}

impl PartialEq for Compound<'_> {
    /// Order-insensitive, because the server's `CompoundTag` is a map.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .entries
                .iter()
                .all(|(name, value)| other.get(name) == Some(value))
    }
}

impl<'a> FromIterator<(Cow<'a, str>, Tag<'a>)> for Compound<'a> {
    fn from_iter<I: IntoIterator<Item = (Cow<'a, str>, Tag<'a>)>>(iter: I) -> Self {
        let mut compound = Self::new();
        for (name, value) in iter {
            compound.insert(name, value);
        }
        compound
    }
}

impl Encode for Compound<'_> {
    /// Write the compound as a whole tag, which is `ByteBufCodecs.compoundTagCodec`.
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(TAG_COMPOUND);
        self.write_payload(writer, 0)
    }
}

impl<'a> Decode<'a> for Compound<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        match Tag::decode(reader)? {
            Tag::Compound(compound) => Ok(compound),
            other => Err(Error::WrongTagType {
                field: "root",
                expected: "TAG_Compound",
                found: other.id(),
            }),
        }
    }
}

/// A sequence of tags.
///
/// The wire form still names one element type, but since 1.21.5 a list whose
/// elements disagree is written as a list of compounds with each element boxed
/// under the empty key. `ListTag.identifyRawElementType`, `wrapIfNeeded` and
/// `addAndUnwrap` between them make that invisible to callers, and so does
/// this: push whatever you like and the encoding sorts itself out.
///
/// The boxing is not free of edge cases. A list of compounds where one of them
/// happens to be a single entry under the empty key gets that element boxed
/// too, so it survives the unboxing on the far side; `isWrapper` is the check
/// that catches it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct List<'a> {
    elements: Vec<Tag<'a>>,
}

/// The key `ListTag` boxes mismatched elements under (`ListTag.WRAPPER_MARKER`).
const WRAPPER_MARKER: &str = "";

impl<'a> List<'a> {
    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    /// Number of elements.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.elements.len()
    }

    /// True when there are no elements.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Append an element.
    pub fn push(&mut self, tag: Tag<'a>) {
        self.elements.push(tag);
    }

    /// The elements in order.
    #[must_use]
    pub fn as_slice(&self) -> &[Tag<'a>] {
        &self.elements
    }

    /// The element type this list would be written with
    /// (`ListTag.identifyRawElementType`).
    ///
    /// `TAG_End` for an empty list, the shared type for a uniform one, and
    /// `TAG_Compound` for a mixed one whatever the elements actually are.
    #[must_use]
    pub fn element_type(&self) -> u8 {
        let mut uniform = TAG_END;
        for element in &self.elements {
            let id = element.id();
            if uniform == TAG_END {
                uniform = id;
            } else if uniform != id {
                return TAG_COMPOUND;
            }
        }
        uniform
    }

    fn write_payload(&self, writer: &mut Writer, depth: usize) -> Result<()> {
        if depth >= MAX_DEPTH {
            return Err(Error::NbtTooDeep);
        }
        let element_type = self.element_type();
        writer.u8(element_type);
        writer.i32(length_prefix(self.elements.len())?);
        for element in &self.elements {
            if element_type == TAG_COMPOUND && needs_wrapping(element) {
                // Written inline rather than by building the wrapper compound,
                // which would clone the element for no reason.
                writer.u8(element.id());
                mutf8::write(writer, WRAPPER_MARKER)?;
                element.write_payload(writer, depth + 2)?;
                writer.u8(TAG_END);
            } else {
                element.write_payload(writer, depth + 1)?;
            }
        }
        Ok(())
    }

    /// `ListTag.addAndUnwrap`: undo the boxing a mixed list applied.
    fn push_unwrapped(&mut self, tag: Tag<'a>) {
        match tag {
            Tag::Compound(mut compound) if compound.len() == 1 => {
                match compound.remove(WRAPPER_MARKER) {
                    Some(inner) => self.elements.push(inner),
                    None => self.elements.push(Tag::Compound(compound)),
                }
            }
            other => self.elements.push(other),
        }
    }
}

impl<'a> FromIterator<Tag<'a>> for List<'a> {
    fn from_iter<I: IntoIterator<Item = Tag<'a>>>(iter: I) -> Self {
        Self {
            elements: iter.into_iter().collect(),
        }
    }
}

/// `ListTag.wrapIfNeeded` with the element type already known to be compound.
fn needs_wrapping(tag: &Tag<'_>) -> bool {
    match tag {
        // A one-entry compound keyed on the marker is indistinguishable from a
        // box, so it gets one of its own to survive the round trip.
        Tag::Compound(compound) => compound.len() == 1 && compound.get(WRAPPER_MARKER).is_some(),
        _ => true,
    }
}

/// Read one tag of a known type without recursing.
///
/// `CompoundTag.load` and `ListTag.load` recurse in Java and lean on
/// `NbtAccounter.pushDepth` to stop at 512. Transcribing that shape into Rust
/// puts a much larger frame on a much smaller stack: a straight recursive
/// decoder overflows a test thread's two megabytes somewhere short of 512
/// levels, so the guard aborts the process instead of returning an error, and
/// the input that does it is three bytes per level. An explicit stack keeps
/// the limit exactly where Mojang put it and makes it an error again.
fn read_nested<'a>(reader: &mut Reader<'a>, root: u8) -> Result<Tag<'a>> {
    let mut stack: Vec<Frame<'a>> = Vec::new();
    let mut wanted = root;

    loop {
        // Read the value the innermost unfinished container is waiting for,
        // descending for as long as that value is itself a container.
        let mut value = loop {
            match wanted {
                TAG_COMPOUND => {
                    push_depth(&stack)?;
                    let Some(name) = read_entry_name(reader, &mut wanted)? else {
                        break Tag::Compound(Compound::new());
                    };
                    stack.push(Frame::Compound {
                        compound: Compound::new(),
                        name,
                    });
                }
                TAG_LIST => {
                    push_depth(&stack)?;
                    let element_type = reader.u8()?;
                    let count = read_count(reader)?;
                    if element_type == TAG_END && count > 0 {
                        // `loadList` throws "Missing type on ListTag" here, and
                        // it does so before looking at the input, so this check
                        // has to come before the one below.
                        return Err(Error::InvalidTagType(TAG_END));
                    }
                    if count == 0 {
                        break Tag::List(List::new());
                    }
                    // Every element costs at least one byte, so the input that
                    // is left bounds the count and the reservation cannot be
                    // inflated.
                    ensure_available(reader, count, 1)?;
                    stack.push(Frame::List {
                        list: List {
                            elements: Vec::with_capacity(count),
                        },
                        element_type,
                        remaining: count,
                    });
                    wanted = element_type;
                }
                leaf => break Tag::read_leaf(reader, leaf)?,
            }
        };

        // Hand the finished value to its parent, completing parents as they
        // fill, until one of them still wants another value.
        loop {
            let Some(frame) = stack.last_mut() else {
                return Ok(value);
            };
            match frame {
                Frame::Compound { compound, name } => {
                    let key = std::mem::replace(name, Cow::Borrowed(""));
                    compound.insert(key, value);
                    match read_entry_name(reader, &mut wanted)? {
                        Some(next) => {
                            *name = next;
                            break;
                        }
                        None => value = stack.pop().expect("frame present").finish(),
                    }
                }
                Frame::List {
                    list,
                    element_type,
                    remaining,
                } => {
                    list.push_unwrapped(value);
                    *remaining -= 1;
                    if *remaining > 0 {
                        wanted = *element_type;
                        break;
                    }
                    value = stack.pop().expect("frame present").finish();
                }
            }
        }
    }
}

/// A container waiting for the value being read to be handed back to it.
enum Frame<'a> {
    Compound {
        compound: Compound<'a>,
        /// Key the value under construction belongs to.
        name: Cow<'a, str>,
    },
    List {
        list: List<'a>,
        element_type: u8,
        remaining: usize,
    },
}

impl<'a> Frame<'a> {
    fn finish(self) -> Tag<'a> {
        match self {
            Self::Compound { compound, .. } => Tag::Compound(compound),
            Self::List { list, .. } => Tag::List(list),
        }
    }
}

/// Read a compound entry header, or `None` at the terminating `TAG_End`.
fn read_entry_name<'a>(reader: &mut Reader<'a>, wanted: &mut u8) -> Result<Option<Cow<'a, str>>> {
    let id = reader.u8()?;
    if id == TAG_END {
        return Ok(None);
    }
    *wanted = id;
    mutf8::read(reader).map(Some)
}

/// `NbtAccounter.pushDepth`, with the stack's own height standing in for its
/// counter.
const fn push_depth(stack: &[Frame<'_>]) -> Result<()> {
    if stack.len() >= MAX_DEPTH {
        return Err(Error::NbtTooDeep);
    }
    Ok(())
}

/// A length that is about to be written as a signed int.
fn length_prefix(length: usize) -> Result<i32> {
    i32::try_from(length).map_err(|_| Error::NegativeLength(-1))
}

/// Read an element count and refuse one the remaining input cannot supply.
///
/// This is what stands in for `NbtAccounter`: the count is attacker-controlled
/// and feeds a reservation, so it is checked against bytes actually present
/// before anything is allocated.
fn counted(reader: &mut Reader<'_>, stride: usize) -> Result<usize> {
    let count = read_count(reader)?;
    ensure_available(reader, count, stride)?;
    Ok(count)
}

/// Read an element count, rejecting a negative one as `readListCount` does.
fn read_count(reader: &mut Reader<'_>) -> Result<usize> {
    let count = reader.i32()?;
    usize::try_from(count).map_err(|_| Error::NegativeLength(count))
}

const fn ensure_available(reader: &Reader<'_>, count: usize, stride: usize) -> Result<()> {
    let needed = count.saturating_mul(stride);
    if needed > reader.remaining_len() {
        return Err(Error::UnexpectedEof {
            needed,
            available: reader.remaining_len(),
        });
    }
    Ok(())
}
