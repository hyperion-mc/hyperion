//! Items and the data component system.
//!
//! Since 1.20.5 an item is a count, an item type, and a *patch* over the
//! component map its type already implies: components set, and components
//! removed. There are 111 component types in 26.2, listed in the
//! `minecraft:data_component_type` registry, and each one has its own codec.
//!
//! # Why values are kept as bytes
//!
//! `DataComponentPatch.STREAM_CODEC` writes a component type id followed
//! immediately by that type's value. There is no length between them, so a
//! reader cannot skip a component it does not understand -- it cannot find
//! where the value ends without replaying that type's layout. Guessing
//! desynchronises everything after it, and dropping the component corrupts the
//! item.
//!
//! This module therefore separates two questions that are usually conflated:
//!
//! 1. *Where does this value end?* Answered for all 111 types by
//!    [`ComponentType::shape`], a table transcribed from the server's own
//!    `StreamCodec` compositions and walked by [`shape::Shape`].
//! 2. *What does this value mean?* Answered for the subset in [`payload`].
//!
//! A patch holds each value as the exact bytes it occupied, so a component this
//! crate does not interpret still re-encodes byte-identically. Losslessness
//! comes from the first question alone; the second is a convenience on top.
//!
//! # NBT
//!
//! Some shapes bottom out in an NBT tag, and text components do too -- since
//! 1.20.5 `ComponentSerialization.STREAM_CODEC` writes a component as NBT
//! rather than as JSON. Measuring a tag is the one thing this module cannot do
//! from primitives, so it takes an [`nbt::NbtScan`] implementation as a
//! parameter rather than growing its own NBT reader.

pub mod nbt;
pub mod payload;
pub mod shape;

mod component_type;
mod patch;
mod stack;

pub use component_type::ComponentType;
pub use patch::DataComponentPatch;
pub use shape::Shape;
pub use stack::{ItemStack, Slot};
