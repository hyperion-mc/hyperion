//! Which codec each tracked value is written with, for Minecraft 26.2.
//!
//! A tracked value carries its serializer id on the wire, and the client uses
//! that id to decide how many bytes to read and which field of the entity to
//! put them in. The id is an index into a registration-ordered table, so it
//! moves whenever Mojang inserts a serializer: four of the ten this server
//! sends shifted between 1.20.1 and protocol 776, and thirty are new. Sending
//! the old number does not fail, it writes a float into a field that expects a
//! long.
//!
//! The numbers themselves live in
//! [`hyperion_minecraft_proto::packets::play::entity::EntityDataSerializer`],
//! read off the pinned server jar. What is here is the mapping from the Rust
//! type a component holds to the serializer that type is sent with, plus the
//! encoding, because several of those types are foreign and cannot carry the
//! proto crate's `Encode` implementation themselves.

use glam::{Quat, Vec3};
use hyperion_minecraft_proto::{
    Encode, Result, VarInt, Writer, item::Slot, packets::play::entity::EntityDataSerializer,
    text::Component,
};

use crate::simulation::metadata::entity::{HumanoidArm, Pose};

/// A value that can be sent as entity tracked data.
pub trait MetadataType {
    /// The serializer the field was declared with.
    ///
    /// The client rejects a value whose serializer does not match the one its
    /// own accessor was defined with, so this and the field index have to move
    /// together.
    const SERIALIZER: EntityDataSerializer;

    /// Write the value the way that serializer's codec writes it.
    ///
    /// # Errors
    /// Returns an error when the value exceeds a protocol limit.
    fn write(&self, writer: &mut Writer) -> Result<()>;
}

/// The straightforward cases, where the value already encodes itself.
macro_rules! delegating_metadata_type {
    ($($serializer:ident => $type:ty),* $(,)?) => {
        $(
            impl MetadataType for $type {
                const SERIALIZER: EntityDataSerializer = EntityDataSerializer::$serializer;

                fn write(&self, writer: &mut Writer) -> Result<()> {
                    Encode::encode(self, writer)
                }
            }
        )*
    };
}

delegating_metadata_type! {
    Byte => u8,
    Int => VarInt,
    Float => f32,
    Boolean => bool,
    ItemStack => Slot<'static>,
    OptionalComponent => Option<Component<'static>>,
}

/// `ByteBufCodecs.idMapper(Block.BLOCK_STATE_REGISTRY)`.
///
/// The number is an index into one global table of all 32366 states, and the
/// numbering is dense but arbitrary, so it moves with almost every game
/// version. [`hyperion_minecraft_proto::block_state`] resolves a name to the id
/// this protocol sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockStateId(pub u32);

impl MetadataType for BlockStateId {
    const SERIALIZER: EntityDataSerializer = EntityDataSerializer::BlockState;

    fn write(&self, writer: &mut Writer) -> Result<()> {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "32366 states, so no id reaches the sign bit"
        )]
        writer.var_int(self.0 as i32);
        Ok(())
    }
}

impl MetadataType for Pose {
    const SERIALIZER: EntityDataSerializer = EntityDataSerializer::Pose;

    fn write(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(*self as i32);
        Ok(())
    }
}

impl MetadataType for HumanoidArm {
    const SERIALIZER: EntityDataSerializer = EntityDataSerializer::HumanoidArm;

    fn write(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(*self as i32);
        Ok(())
    }
}

impl MetadataType for Vec3 {
    const SERIALIZER: EntityDataSerializer = EntityDataSerializer::Vector3;

    fn write(&self, writer: &mut Writer) -> Result<()> {
        // `ByteBufCodecs.VECTOR3F`: three big-endian floats, no length.
        writer.f32(self.x);
        writer.f32(self.y);
        writer.f32(self.z);
        Ok(())
    }
}

impl MetadataType for Quat {
    const SERIALIZER: EntityDataSerializer = EntityDataSerializer::Quaternion;

    fn write(&self, writer: &mut Writer) -> Result<()> {
        // `ByteBufCodecs.QUATERNIONF`: x, y, z, w in that order.
        writer.f32(self.x);
        writer.f32(self.y);
        writer.f32(self.z);
        writer.f32(self.w);
        Ok(())
    }
}

/// Borrows a tracked value so it can be written through [`MetadataType`].
///
/// [`MetadataType`] exists because `glam::Vec3` and the rest are foreign types
/// that cannot carry the proto crate's `Encode`; this is the adapter back, so
/// the run builder stays a plain `Encode` consumer.
pub(crate) struct Tracked<'a, T>(pub &'a T);

impl<T: MetadataType> Encode for Tracked<'_, T> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.0.write(writer)
    }
}
