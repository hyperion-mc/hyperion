//! Wire shapes, and the walker that measures a value without interpreting it.
//!
//! A data component is written as a type id followed immediately by its value,
//! with no length between them (`DataComponentPatch.STREAM_CODEC`). Skipping a
//! value you do not understand is therefore impossible: the next byte after it
//! is only findable by replaying that value's own layout. A proxy that guesses
//! wrong here does not drop one component, it desynchronises the rest of the
//! packet.
//!
//! So every component type carries a [`Shape`], transcribed from its
//! `StreamCodec` in the server sources, and one walker measures all of them.
//! Describing the layouts as data rather than as 111 hand-written readers is
//! what keeps them checkable against the jar line by line.

use crate::{
    Error, Reader, Result,
    item::{ComponentType, nbt::NbtScan, patch::DataComponentPatch},
};

/// How a value is laid out on the wire.
///
/// The variants correspond to the combinators in
/// `net.minecraft.network.codec.ByteBufCodecs`, so a shape reads against the
/// `StreamCodec` it was transcribed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Zero bytes. `StreamCodec.unit`, used by marker components.
    Unit,
    /// One byte. `ByteBufCodecs.BOOL` and `BYTE`.
    Byte,
    /// Four bytes. `ByteBufCodecs.INT` and `FLOAT`.
    Int,
    /// Eight bytes. `ByteBufCodecs.LONG` and `DOUBLE`, and the packed
    /// `BlockPos`.
    Long,
    /// Sixteen bytes. `UUIDUtil.STREAM_CODEC`, two big-endian longs.
    Uuid,
    /// `ByteBufCodecs.VAR_INT`. Also every registry id, and every enum written
    /// through `idMapper`.
    VarInt,
    /// `ByteBufCodecs.STRING_UTF8`, `Identifier.STREAM_CODEC`, and
    /// `ResourceKey.streamCodec`, which is an `Identifier` underneath.
    ///
    /// Per-field character limits are not modelled: they change what the server
    /// accepts, not where the value ends, and enforcing a limit the peer does
    /// not would drop traffic the server would have taken.
    Str,
    /// One network NBT tag. Text components reach here too: since 1.20.5
    /// `ComponentSerialization.STREAM_CODEC` is `fromCodecWithRegistries`,
    /// which writes the component as NBT rather than as JSON.
    Nbt,
    /// A nested `DataComponentPatch`, as `ItemStackTemplate.STREAM_CODEC`
    /// carries for the stacks inside a bundle or a shulker box.
    Patch,
    /// `TypedDataComponent.STREAM_CODEC`: a component type id followed by that
    /// type's value. The only place a shape is chosen at read time.
    TypedComponent,
    /// `ByteBufCodecs.optional`: a boolean, then the value when it is true.
    Optional(&'static Self),
    /// `ByteBufCodecs.list` / `collection`: a `VarInt` count, then that many
    /// elements.
    List(&'static Self),
    /// `StreamCodec.composite`: each field in declaration order.
    Seq(&'static [Self]),
    /// `ByteBufCodecs.either`: a boolean, then the left shape when it is true.
    Either(&'static Self, &'static Self),
    /// `ByteBufCodecs.holder`: a `VarInt`; zero means the value follows inline,
    /// anything else is `id - 1` in the registry and carries no payload.
    Holder(&'static Self),
    /// `ByteBufCodecs.holderSet`: a `VarInt` `n`; zero means a tag name
    /// follows, otherwise `n - 1` registry ids do.
    HolderSet,
    /// `ByteBufCodecs.map`: a `VarInt` count, then that many key/value pairs.
    Map(&'static Self, &'static Self),
    /// `StreamCodec.dispatch`: a `VarInt` selector, then the shape it selects.
    Dispatch {
        /// One shape per selector value, indexed by the selector.
        variants: &'static [Self],
        /// What the server does with a selector past the end of `variants`.
        out_of_range: OutOfRange,
    },
}

/// How a dispatch selector outside the known range is treated.
///
/// Both behaviours exist in the protocol and they are not interchangeable: one
/// consumes bytes where the other consumes none, so picking the wrong one
/// desynchronises the read rather than merely mislabelling a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutOfRange {
    /// The selector came from a registry lookup, which throws on an unknown id.
    Reject,
    /// The selector came from `ByIdMap.continuous(..., OutOfBoundsStrategy.ZERO)`,
    /// which silently substitutes the first variant.
    Clamp,
}

/// Cap on nesting depth while measuring a value.
///
/// Shapes recurse: a bundle holds stacks, whose components can hold further
/// bundles, and `MobEffectInstance` nests a hidden effect inside itself. The
/// walker is recursive, so without a cap a hostile client can turn a deeply
/// nested item into a stack overflow rather than a decode error. 32 is far past
/// anything vanilla produces and shallow enough that the deepest walk cannot
/// exhaust a thread's stack.
pub const MAX_DEPTH: u32 = 32;

/// Reject a count that cannot fit in the bytes that remain.
///
/// `ByteBufCodecs.readCount` caps most collections at `Integer.MAX_VALUE` and
/// then allocates `min(count, 65536)`, discovering the lie when the buffer runs
/// out. This walker never allocates per element, so it can be stricter for
/// free: every element costs at least one byte, so a count larger than the
/// remaining length is already a lie.
pub(crate) fn check_count(count: i32, remaining: usize) -> Result<usize> {
    let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
    if count > remaining {
        return Err(Error::UnexpectedEof {
            needed: count,
            available: remaining,
        });
    }
    Ok(count)
}

impl Shape {
    /// Advance `reader` past one value of this shape.
    ///
    /// # Errors
    /// Returns an error on truncated input, a malformed NBT tag, a negative
    /// count, an unknown component type, or nesting past [`MAX_DEPTH`].
    pub fn skip(&self, reader: &mut Reader<'_>, nbt: &impl NbtScan) -> Result<()> {
        self.skip_at_depth(reader, nbt, 0)
    }

    /// Consume one value of this shape and return exactly the bytes it spanned.
    ///
    /// This is what makes a component this crate does not model still safe to
    /// forward: the span is re-emitted verbatim, so nothing is reinterpreted on
    /// the way through.
    ///
    /// # Errors
    /// See [`Shape::skip`].
    pub fn measure<'a>(&self, reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<&'a [u8]> {
        self.measure_at_depth(reader, nbt, 0)
    }

    pub(crate) fn measure_at_depth<'a>(
        &self,
        reader: &mut Reader<'a>,
        nbt: &impl NbtScan,
        depth: u32,
    ) -> Result<&'a [u8]> {
        let before = reader.remaining();
        self.skip_at_depth(reader, nbt, depth)?;
        let consumed = before.len() - reader.remaining_len();
        Ok(&before[..consumed])
    }

    fn skip_at_depth(&self, reader: &mut Reader<'_>, nbt: &impl NbtScan, depth: u32) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(Error::DepthLimitExceeded(MAX_DEPTH));
        }
        let deeper = depth + 1;
        match self {
            Self::Unit => {}
            Self::Byte => {
                reader.u8()?;
            }
            Self::Int => {
                reader.take(4)?;
            }
            Self::Long => {
                reader.take(8)?;
            }
            Self::Uuid => {
                reader.take(16)?;
            }
            Self::VarInt => {
                reader.var_int()?;
            }
            Self::Str => {
                reader.string()?;
            }
            Self::Nbt => {
                let length = nbt.tag_len(reader.remaining())?;
                reader.take(length)?;
            }
            Self::Patch => {
                DataComponentPatch::decode_at_depth(reader, nbt, deeper)?;
            }
            Self::TypedComponent => {
                let kind = ComponentType::decode(reader)?;
                kind.shape().skip_at_depth(reader, nbt, deeper)?;
            }
            Self::Optional(inner) => {
                if reader.bool()? {
                    inner.skip_at_depth(reader, nbt, deeper)?;
                }
            }
            Self::List(element) => {
                let count = check_count(reader.var_int()?, reader.remaining_len())?;
                for _ in 0..count {
                    element.skip_at_depth(reader, nbt, deeper)?;
                }
            }
            Self::Seq(fields) => {
                for field in *fields {
                    field.skip_at_depth(reader, nbt, deeper)?;
                }
            }
            Self::Either(left, right) => {
                let chosen = if reader.bool()? { left } else { right };
                chosen.skip_at_depth(reader, nbt, deeper)?;
            }
            Self::Holder(direct) => {
                if reader.var_int()? == 0 {
                    direct.skip_at_depth(reader, nbt, deeper)?;
                }
            }
            Self::HolderSet => {
                let marker = reader.var_int()?;
                if marker == 0 {
                    reader.string()?;
                } else {
                    // The marker is biased by one so that zero can mean "a tag
                    // name follows" instead of "an empty set".
                    let count = check_count(marker - 1, reader.remaining_len())?;
                    for _ in 0..count {
                        reader.var_int()?;
                    }
                }
            }
            Self::Map(key, value) => {
                let count = check_count(reader.var_int()?, reader.remaining_len())?;
                for _ in 0..count {
                    key.skip_at_depth(reader, nbt, deeper)?;
                    value.skip_at_depth(reader, nbt, deeper)?;
                }
            }
            Self::Dispatch {
                variants,
                out_of_range,
            } => {
                let selector = reader.var_int()?;
                let chosen = usize::try_from(selector)
                    .ok()
                    .and_then(|index| variants.get(index));
                let chosen = match (chosen, out_of_range) {
                    (Some(shape), _) => shape,
                    (None, OutOfRange::Clamp) => &variants[0],
                    (None, OutOfRange::Reject) => {
                        return Err(Error::InvalidEnum {
                            name: "dispatch selector",
                            value: selector,
                        });
                    }
                };
                chosen.skip_at_depth(reader, nbt, deeper)?;
            }
        }
        Ok(())
    }
}
