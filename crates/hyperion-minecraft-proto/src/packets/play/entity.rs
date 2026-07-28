//! Spawning, movement and tracked data.
//!
//! Most of this group is mechanical and lives in [`clientbound`]; the names are
//! re-exported here so a reader looking for entity movement finds all of it in
//! one place. Two bodies are hand-written, each because its encoder branches on
//! something the generator refuses to guess at:
//!
//! * [`SetEntityData`] is a run of entries terminated by a sentinel rather than
//!   a counted list, and each entry's length is decided by its serializer.
//! * [`SetEquipment`] packs a continuation bit into the slot byte, so the entry
//!   count is carried by the entries themselves.
//!
//! [`AddEntity`], [`SetEntityMotion`] and [`Interact`] used to be here too, for
//! a reason that was not really theirs: each carries a velocity through
//! [`lp_vec3`], whose byte count depends on the magnitude of the vector, and a
//! single field the extractor could not read as a layout cost the whole packet.
//! `protocol.json` now marks that one field `{"kind": "custom"}` and names this
//! module's codec for it, so all three are generated and only re-exported here.
//!
//! [`DamageEvent`] is hand-written for a different reason: the extractor read
//! its layout in full, but two of its fields are the same anonymous `output`
//! in the decompiled source, so the generator had no names to give them.
//!
//! [`clientbound`]: super::clientbound

pub use super::{
    clientbound::{
        AddEntity, Animate, EntityEvent, EntityPositionSync, HurtAnimation, MoveEntityPos,
        MoveEntityPosRot, MoveEntityRot, RemoveEntities, RotateHead, SetEntityMotion,
        TeleportEntity, UpdateAttributes, update_attributes,
    },
    serverbound::Interact,
};
use crate::{
    Decode, Encode, Error, Reader, RegistryId, Result, Writer,
    item::{Slot, nbt::NbtScan},
    types::Vec3,
};

// --- rotation -------------------------------------------------------------

/// One byte of rotation, as `Mth.packDegrees` writes it.
///
/// A full turn is 256 steps, and the result is a signed byte, so 180 degrees
/// and -180 degrees are the same value. The floor is Mojang's: rounding to
/// nearest instead would be off by one step for most angles, which is a
/// visible twitch on a mob and nothing an encoder would notice.
#[must_use]
pub fn pack_degrees(degrees: f32) -> i8 {
    // `Mth.floor(float)` is `(int) Math.floor(v)`, which saturates rather than
    // wrapping, and the `(byte)` cast then keeps the low eight bits. Rust's
    // float-to-int cast saturates the same way, so the two casts in sequence
    // reproduce Java exactly, including for infinities and NaN.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the narrowing to a byte is the encoding, matching Java's (byte) cast"
    )]
    let whole = (degrees * 256.0 / 360.0).floor() as i32 as i8;
    whole
}

/// The angle a [`pack_degrees`] byte stands for (`Mth.unpackDegrees`).
///
/// The multiply is widened to 32 bits because Java promotes the `byte` to an
/// `int` before it; at 16 bits the largest packed angle overflows.
#[must_use]
pub fn unpack_degrees(packed: i8) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the product is at most 45720, exact in f32"
    )]
    let degrees = (i32::from(packed) * 360) as f32 / 256.0;
    degrees
}

// --- velocity -------------------------------------------------------------

/// `net.minecraft.world.phys.Vec3#LP_STREAM_CODEC`, as a
/// `#[proto(with = ...)]` module.
///
/// The three components share one integer scale, and each is quantised to 15
/// bits of that scale, so a velocity costs six bytes rather than the
/// twenty-four `Vec3.STREAM_CODEC` spends. Two cases fall outside that:
///
/// * A vector shorter than [`ABS_MIN_VALUE`] in every component is a single
///   zero byte, which is the common case for an entity standing still.
/// * A scale above three does not fit the two bits reserved for it, so a
///   continuation bit is set and the remaining scale follows as a `VarInt`.
///
/// Everything here mirrors `net.minecraft.network.LpVec3`.
pub mod lp_vec3 {
    use crate::{Reader, Result, Writer, types::Vec3};

    /// Bits each component is quantised to (`LpVec3.DATA_BITS`).
    const DATA_BITS: u32 = 15;
    /// Largest quantised value (`LpVec3.MAX_QUANTIZED_VALUE`).
    const MAX_QUANTIZED_VALUE: i64 = 32766;
    /// The same bound, for the arithmetic that runs in floating point.
    const MAX_QUANTIZED: f64 = MAX_QUANTIZED_VALUE as f64;
    /// Set in the low byte when the scale continues as a `VarInt`.
    const CONTINUATION_FLAG: i64 = 4;
    /// The two low bits the scale shares with the continuation flag.
    const SCALE_BITS_MASK: i64 = 3;
    /// Bit offset of the packed x component (`LpVec3.X_OFFSET`).
    const X_OFFSET: u32 = 3;
    /// Bit offset of the packed y component (`LpVec3.Y_OFFSET`).
    const Y_OFFSET: u32 = X_OFFSET + DATA_BITS;
    /// Bit offset of the packed z component (`LpVec3.Z_OFFSET`).
    const Z_OFFSET: u32 = Y_OFFSET + DATA_BITS;
    /// Largest magnitude the encoding represents (`LpVec3.ABS_MAX_VALUE`).
    pub const ABS_MAX_VALUE: f64 = 1.717_986_918_3e10;
    /// Below this in every component the vector is sent as a single zero byte
    /// (`LpVec3.ABS_MIN_VALUE`).
    pub const ABS_MIN_VALUE: f64 = 3.051_944_088_384_301e-5;

    /// `LpVec3.sanitize`: NaN becomes zero, and anything past the range clamps.
    fn sanitize(value: f64) -> f64 {
        if value.is_nan() {
            0.0
        } else {
            value.clamp(-ABS_MAX_VALUE, ABS_MAX_VALUE)
        }
    }

    /// `LpVec3.pack`: a value in `[-1, 1]` to 15 bits.
    fn pack(value: f64) -> i64 {
        // `Math.round(double)` is floor(x + 0.5), which differs from Rust's
        // round-half-away-from-zero below zero. The argument here is always in
        // `[0, MAX_QUANTIZED_VALUE]` because the caller has divided by a scale
        // that is at least the largest component, so the two agree; the floor
        // is written out anyway so a future caller with a negative argument
        // does not silently pick up the other rounding.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the argument is bounded by MAX_QUANTIZED_VALUE, well inside i64"
        )]
        #[expect(
            clippy::suboptimal_flops,
            reason = "`mul_add` rounds once where Java rounds twice, which moves the result"
        )]
        let packed = ((value * 0.5 + 0.5) * MAX_QUANTIZED + 0.5).floor() as i64;
        packed
    }

    /// `LpVec3.unpack`: 15 bits back to a value in `[-1, 1]`.
    fn unpack(value: i64) -> f64 {
        let clamped = (value & ((1 << DATA_BITS) - 1)).min(MAX_QUANTIZED_VALUE);
        clamped as f64 * 2.0 / MAX_QUANTIZED - 1.0
    }

    /// Write a velocity in the packed form.
    ///
    /// # Errors
    /// Never fails; the signature matches what `#[proto(with = ...)]` calls.
    pub fn encode(value: &Vec3, writer: &mut Writer) -> Result<()> {
        let x = sanitize(value.x);
        let y = sanitize(value.y);
        let z = sanitize(value.z);

        // `Mth.absMax` twice: the largest component decides the shared scale.
        let chessboard_length = x.abs().max(y.abs()).max(z.abs());
        if chessboard_length < ABS_MIN_VALUE {
            writer.u8(0);
            return Ok(());
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "sanitize has clamped to ABS_MAX_VALUE, far inside i64"
        )]
        let scale = chessboard_length.ceil() as i64;
        let is_partial = (scale & SCALE_BITS_MASK) != scale;
        let markers = if is_partial {
            (scale & SCALE_BITS_MASK) | CONTINUATION_FLAG
        } else {
            scale
        };

        let divisor = scale as f64;
        let buffer = markers
            | (pack(x / divisor) << X_OFFSET)
            | (pack(y / divisor) << Y_OFFSET)
            | (pack(z / divisor) << Z_OFFSET);

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "each cast takes the byte lane the layout assigns it"
        )]
        {
            writer.u8(buffer as u8);
            writer.u8((buffer >> 8) as u8);
            writer.i32((buffer >> 16) as i32);
            if is_partial {
                // Java writes `(int)(scale >> 2)`, so a scale needing all 32
                // bits arrives as a negative `VarInt` and is read back
                // unsigned. Truncating the same way is what keeps the two ends
                // agreeing on such a scale.
                writer.var_int((scale >> 2) as i32);
            }
        }
        Ok(())
    }

    /// Read a velocity in the packed form.
    ///
    /// # Errors
    /// Returns an error on truncated input.
    pub fn decode(reader: &mut Reader<'_>) -> Result<Vec3> {
        let lowest = i64::from(reader.u8()?);
        if lowest == 0 {
            return Ok(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
        }
        let middle = i64::from(reader.u8()?);
        // Java reads these four bytes with `readUnsignedInt`, so a buffer whose
        // top bit is set is a large positive value rather than a negative one.
        let highest = i64::from(reader.i32()?.cast_unsigned());
        let buffer = (highest << 16) | (middle << 8) | lowest;

        let mut scale = lowest & SCALE_BITS_MASK;
        if lowest & CONTINUATION_FLAG == CONTINUATION_FLAG {
            // Likewise `(long) VarInt.read(input) & 0xFFFFFFFFL`: a scale
            // needing all 32 bits arrives as a negative `VarInt`.
            scale |= i64::from(reader.var_int()?.cast_unsigned()) << 2;
        }

        let scale = scale as f64;
        Ok(Vec3 {
            x: unpack(buffer >> X_OFFSET) * scale,
            y: unpack(buffer >> Y_OFFSET) * scale,
            z: unpack(buffer >> Z_OFFSET) * scale,
        })
    }

    /// The value a client reads back after this one has been through the wire.
    ///
    /// The three components share one exponent and keep fifteen bits each, so
    /// how much a small component is rounded depends on how large the largest
    /// one is. A caller that has to know what the client will actually do --
    /// a test asserting a knockback arrived, or a simulation that must stay in
    /// step with the client's -- asks here rather than assuming the value it
    /// sent survived.
    ///
    /// Written as an encode followed by a decode rather than as the rounding
    /// arithmetic a second time. A second copy of a rounding rule is the copy
    /// that drifts; this one is the same code the packet uses, so it cannot
    /// disagree with it.
    ///
    /// # Panics
    /// Never: [`encode`] has no failing path, and its output always decodes.
    #[must_use]
    pub fn quantize(value: &Vec3) -> Vec3 {
        let mut writer = Writer::new();
        encode(value, &mut writer).expect("lp_vec3 encoding has no failing path");
        let bytes = writer.into_vec();
        decode(&mut Reader::new(&bytes)).expect("lp_vec3 decodes its own output")
    }
}

// --- tracked data ---------------------------------------------------------

/// Which codec a tracked value is written with.
///
/// The number is an index into `EntityDataSerializers.SERIALIZERS`, a bimap
/// filled in the order of the `registerSerializer` calls in that class's static
/// initialiser, so it moves whenever Mojang inserts one. Four of these shifted
/// between 1.20.1 and 26.2 and thirty are new, which is why a value written
/// against the old numbering lands in a field of the wrong type rather than
/// failing.
///
/// Hand-written, not generated: `nix/extract-protocol.py` follows packet stream
/// codecs and this table is neither a packet nor reachable from one. The values
/// were read out of the pinned jar by reflecting over `EntityDataSerializers`
/// and calling `getSerializedId` on each field, and the ten that a wire test
/// covers are pinned by fixtures the server itself printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
#[repr(i32)]
pub enum EntityDataSerializer {
    Byte = 0,
    Int = 1,
    Long = 2,
    Float = 3,
    String = 4,
    Component = 5,
    OptionalComponent = 6,
    ItemStack = 7,
    Boolean = 8,
    Rotations = 9,
    BlockPos = 10,
    OptionalBlockPos = 11,
    Direction = 12,
    OptionalLivingEntityReference = 13,
    BlockState = 14,
    OptionalBlockState = 15,
    Particle = 16,
    Particles = 17,
    VillagerData = 18,
    OptionalUnsignedInt = 19,
    Pose = 20,
    CatVariant = 21,
    CatSoundVariant = 22,
    CowVariant = 23,
    CowSoundVariant = 24,
    WolfVariant = 25,
    WolfSoundVariant = 26,
    FrogVariant = 27,
    PigVariant = 28,
    PigSoundVariant = 29,
    ChickenVariant = 30,
    ChickenSoundVariant = 31,
    ZombieNautilusVariant = 32,
    OptionalGlobalPos = 33,
    PaintingVariant = 34,
    SnifferState = 35,
    ArmadilloState = 36,
    CopperGolemState = 37,
    WeatheringCopperState = 38,
    Vector3 = 39,
    Quaternion = 40,
    ResolvableProfile = 41,
    HumanoidArm = 42,
}

impl EntityDataSerializer {
    /// Numeric id as written on the wire.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self as i32
    }
}

/// The `packedItems` run of a [`SetEntityData`], built one value at a time.
///
/// An entry is an index byte, the serializer as a `VarInt`, then that
/// serializer's value with nothing between them, so a reader that does not know
/// the serializer cannot find where a value ends. Holding the run as bytes is
/// therefore the only lossless shape short of modelling all 43 serializers, and
/// it is also what a server accumulating changes over a tick already has.
///
/// The `0xFF` terminator belongs to the packet rather than to the run, so it is
/// written by [`SetEntityData`] and is never in here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataValues {
    bytes: Vec<u8>,
}

impl DataValues {
    /// An empty run.
    #[must_use]
    pub const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Append one value (`SynchedEntityData.DataValue.write`).
    ///
    /// `index` is the entity's own field index, which depends on the entity's
    /// class; `serializer` must be the one that field was declared with, since
    /// the client rejects a value whose serializer does not match.
    ///
    /// # Errors
    /// Returns whatever encoding `value` fails with.
    pub fn push<T: Encode + ?Sized>(
        &mut self,
        index: u8,
        serializer: EntityDataSerializer,
        value: &T,
    ) -> Result<()> {
        // A run is appended to a `Writer` rather than to `self.bytes` directly
        // so that a value which fails halfway cannot leave a partial entry
        // behind, which would desynchronise every entry after it.
        let mut writer = Writer::new();
        writer.u8(index);
        writer.var_int(serializer.to_raw());
        value.encode(&mut writer)?;
        self.bytes.extend_from_slice(writer.as_slice());
        Ok(())
    }

    /// True when nothing has been pushed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The run as it will be written, without the terminator.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Drop every entry, keeping the allocation.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }
}

/// `minecraft:set_entity_data`, sent clientbound as play id 99.
///
/// Layout from `net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket#STREAM_CODEC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetEntityData<'a> {
    /// Network id of the entity whose tracked data this is.
    pub id: i32,
    /// Entries, without the terminator; see [`DataValues`].
    pub packed_items: &'a [u8],
}

/// `ClientboundSetEntityDataPacket.EOF_MARKER`.
const ENTITY_DATA_EOF: u8 = 0xFF;

impl Encode for SetEntityData<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.id);
        writer.raw(self.packed_items);
        writer.u8(ENTITY_DATA_EOF);
        Ok(())
    }
}

impl<'a> Decode<'a> for SetEntityData<'a> {
    /// Reads the entries as bytes rather than as values.
    ///
    /// Finding the end of an entry needs its serializer's layout, so the run
    /// cannot be walked without a table of all 43. The frame already delimits
    /// the body, so the terminator is found from the end instead: everything up
    /// to the final byte is the run, and that byte has to be the marker.
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let id = reader.var_int()?;
        let rest = reader.remaining();
        let (&last, packed_items) = rest.split_last().ok_or(Error::UnexpectedEof {
            needed: 1,
            available: 0,
        })?;
        if last != ENTITY_DATA_EOF {
            return Err(Error::MissingField(
                "ClientboundSetEntityDataPacket terminator",
            ));
        }
        reader.take(rest.len())?;
        Ok(Self { id, packed_items })
    }
}

// --- equipment ------------------------------------------------------------

/// Where a piece of equipment is worn.
///
/// Generated from `net.minecraft.world.entity.EquipmentSlot`, numbered by
/// `ordinal()` -- which is what [`SetEquipment`] writes and what its reader
/// indexes `EquipmentSlot.VALUES` with. It is *not* that class's own
/// `STREAM_CODEC`, which sends the `id` field: the two agree on six of the
/// eight constants and disagree on `OffHand` and `Head`, so using one where
/// the other belongs silently moves a helmet onto the wrong entity part. The
/// generated module says the same thing at its own top.
///
/// This used to be written out here *and* in [`crate::packets::play::inventory`],
/// two copies of one table with two different accessor names.
pub use crate::generated::java_enum::EquipmentSlot;

/// The slot's ordinal as the one byte `SetEquipment` writes it in.
///
/// [`EquipmentSlot::id`] is the `i32` every other site wants; this packet
/// packs the number into a byte alongside a continuation bit, so the narrowing
/// happens once, here, against the `#[repr(u8)]` the generator emitted.
pub(crate) const fn slot_byte(slot: EquipmentSlot) -> u8 {
    slot as u8
}

/// One slot of a [`SetEquipment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquipmentEntry<'a> {
    /// Which slot the stack goes in.
    pub slot: EquipmentSlot,
    /// What is in it; [`Slot::Empty`] clears the slot.
    pub item: Slot<'a>,
}

/// `minecraft:set_equipment`, sent clientbound as play id 102.
///
/// Layout from `net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket#STREAM_CODEC`.
///
/// The entries are not counted. Each slot byte carries a continuation bit
/// (`CONTINUE_MASK`, `0x80`) that is set on every entry but the last, and the
/// reader is a do-while loop, so a packet with no entries is unreadable rather
/// than empty: it would consume the slot byte of whatever followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetEquipment<'a> {
    /// Network id of the entity being equipped.
    pub entity: i32,
    /// At least one slot.
    pub slots: Vec<EquipmentEntry<'a>>,
}

/// `ClientboundSetEquipmentPacket.CONTINUE_MASK`, set on every slot byte but
/// the last.
const EQUIPMENT_CONTINUE_MASK: u8 = 0x80;

impl SetEquipment<'_> {
    /// Read a packet, measuring item components with `nbt`.
    ///
    /// A component value is not length-prefixed, so an item stack can only be
    /// read by walking each component's shape, and one of those shapes is an
    /// NBT tag. See [`crate::item`].
    ///
    /// # Errors
    /// Returns an error on truncated input or a malformed item stack.
    pub fn decode<'a>(reader: &mut Reader<'a>, nbt: &impl NbtScan) -> Result<SetEquipment<'a>> {
        let entity = reader.var_int()?;
        let mut slots = Vec::new();
        loop {
            let raw = reader.u8()?;
            let ordinal = raw & !EQUIPMENT_CONTINUE_MASK;
            let slot =
                EquipmentSlot::from_id(i32::from(ordinal)).ok_or_else(|| Error::InvalidEnum {
                    name: "EquipmentSlot",
                    value: i32::from(ordinal),
                })?;
            let item = Slot::decode(reader, nbt)?;
            slots.push(EquipmentEntry { slot, item });
            if raw & EQUIPMENT_CONTINUE_MASK == 0 {
                break;
            }
        }
        Ok(SetEquipment { entity, slots })
    }
}

impl Encode for SetEquipment<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let Some((last, leading)) = self.slots.split_last() else {
            return Err(Error::MissingField("ClientboundSetEquipmentPacket.slots"));
        };
        writer.var_int(self.entity);
        for entry in leading {
            writer.u8(slot_byte(entry.slot) | EQUIPMENT_CONTINUE_MASK);
            entry.item.encode(writer)?;
        }
        writer.u8(slot_byte(last.slot));
        last.item.encode(writer)
    }
}

// --- damage ---------------------------------------------------------------

/// An entity id that may be absent, written as `id + 1`
/// (`ClientboundDamageEventPacket.writeOptionalEntityId`).
///
/// Zero means absent, so the boolean an [`Option`] would otherwise cost is
/// folded into the value. Mojang spells absence as `-1` rather than as a null,
/// which is why the helper is `+ 1` rather than a presence flag.
mod optional_entity_id {
    use crate::{Reader, Result, Writer};

    /// Write `id + 1`, or zero when absent.
    ///
    /// # Errors
    /// Never fails; the signature matches what `#[proto(with = ...)]` calls.
    #[expect(
        clippy::unnecessary_wraps,
        clippy::ref_option,
        clippy::trivially_copy_pass_by_ref,
        reason = "`#[proto(with = ...)]` hands the encoder a reference to the field"
    )]
    pub fn encode(value: &Option<i32>, writer: &mut Writer) -> Result<()> {
        writer.var_int(value.map_or(0, |id| id.wrapping_add(1)));
        Ok(())
    }

    /// Read an id, mapping zero to absence.
    ///
    /// # Errors
    /// Returns an error on truncated input.
    pub fn decode(reader: &mut Reader<'_>) -> Result<Option<i32>> {
        let raw = reader.var_int()?;
        Ok((raw != 0).then(|| raw.wrapping_sub(1)))
    }
}

/// `minecraft:damage_event`, sent clientbound as play id 25.
///
/// Layout from `net.minecraft.network.protocol.game.ClientboundDamageEventPacket#STREAM_CODEC`.
#[derive(Debug, Clone, Copy, PartialEq, Encode, Decode)]
pub struct DamageEvent {
    #[proto(varint)]
    pub entity_id: i32,
    /// Index into `minecraft:damage_type`.
    pub source_type: RegistryId,
    /// The entity ultimately responsible, such as the archer.
    #[proto(with = optional_entity_id)]
    pub source_cause_id: Option<i32>,
    /// The entity that landed the hit, such as the arrow.
    #[proto(with = optional_entity_id)]
    pub source_direct_id: Option<i32>,
    /// Where the damage came from, for sources with no entity behind them.
    pub source_position: Option<Vec3>,
}
