//! Entity tracked data, as protocol 776 sends it.
//!
//! # The indices are hand-written, not generated
//!
//! A field index comes from `SynchedEntityData.defineId`, which counts the
//! calls in each entity class and offsets each class by its superclasses'
//! total. That numbering never appears on the wire, so
//! `nix/extract-protocol.py` -- which follows stream codecs from a packet down
//! to the bytes -- cannot reach it, and the tables in the modules below were
//! instead read out of the pinned jar by reflection and transcribed here.
//!
//! Making them generated means adding a pass that loads the server classes,
//! walks each entity class's static `EntityDataAccessor` fields, and emits
//! `id()` and `getSerializedId(serializer())` for each. That is the same shape
//! as the existing `nix/java/VanillaEncoder.java` harness and belongs beside
//! it, feeding a table under `src/generated` with the usual staleness check.
//! Until then a Mojang change to a field index is a silently wrong mob rather
//! than a build failure.

use std::fmt::Debug;

use flecs_ecs::{
    core::{
        ComponentId, Entity, EntityView, EntityViewGet, IdOperations, SystemAPI, World,
        WorldProvider, flecs, id,
    },
    macros::Component,
};
use hyperion_minecraft_proto::packets::play::entity::DataValues;

use crate::{
    Prev,
    simulation::metadata::{
        entity::{EntityFlags, Pose},
        player::MainHand,
        r#type::Tracked,
    },
};

pub mod arrow;
pub mod block_display;
pub mod display;
pub mod entity;
pub mod item;
pub mod living_entity;
pub mod player;

#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct MetadataPrefabs {
    pub entity_base: Entity,

    pub arrow_base: Entity,

    pub display_base: Entity,
    pub block_display_base: Entity,

    pub item_base: Entity,

    pub living_entity_base: Entity,
    pub player_base: Entity,
}

fn component_and_track<T>(world: &World) -> fn(&mut EntityView<'_>)
where
    T: ComponentId
        + Clone
        + PartialEq
        + Metadata
        + Default
        + flecs_ecs::core::DataComponent
        + Debug,
{
    world.component::<T>();
    let type_name = core::any::type_name::<T>();

    let system_name = format!("exchange_{type_name}").leak();

    world
        .system_named::<(
            &mut (Prev, T),       //            (0)
            &mut T,               //                  (1)
            &mut MetadataChanges, //     (2)
        )>(system_name)
        .kind(id::<flecs::pipeline::OnUpdate>())
        .each(|(prev, current, metadata_changes)| {
            if prev != current {
                metadata_changes.encode(current.clone());
                prev.clone_from(current);
            }
        });

    |view: &mut EntityView<'_>| {
        view.set_pair::<Prev, _>(T::default()).set(T::default());
    }
}

trait EntityViewExt {
    fn component_and_track<T>(self) -> Self
    where
        T: ComponentId
            + Clone
            + PartialEq
            + Metadata
            + Default
            + flecs_ecs::core::DataComponent
            + Debug;
}

impl EntityViewExt for EntityView<'_> {
    fn component_and_track<T>(mut self) -> Self
    where
        T: ComponentId
            + Clone
            + PartialEq
            + Metadata
            + Default
            + flecs_ecs::core::DataComponent
            + Debug,
    {
        let world = self.world();
        // todo: how this possible exclusive mut
        component_and_track::<T>(&world)(&mut self);
        self
    }
}

#[must_use]
pub fn register_prefabs(world: &World) -> MetadataPrefabs {
    world.component::<MetadataChanges>();

    // these two are hand-written (not produced by `define_and_register_components!`) and their
    // field types are all registered, so they can carry flecs reflection data.
    world.component::<EntityFlags>().meta();
    world.component::<Pose>().meta();

    let entity_base = entity::register_prefab(world, None)
        .add(id::<MetadataChanges>())
        .component_and_track::<EntityFlags>()
        .component_and_track::<Pose>()
        .id();

    // Arrows carry one tracked field of their own, `IN_GROUND`, and it is what
    // stops a client dead-reckoning an arrow through the wall the server
    // stopped it at: `AbstractArrow.tick` returns before moving whenever it is
    // set (`AbstractArrow.java:184-200`).
    let arrow_base = arrow::register_prefab(world, Some(entity_base)).id();

    let display_base = display::register_prefab(world, Some(entity_base)).id();
    let block_display_base = block_display::register_prefab(world, Some(display_base)).id();

    let item_base = item::register_prefab(world, Some(entity_base)).id();

    let living_entity_base = living_entity::register_prefab(world, Some(entity_base)).id();
    let player_base = player::register_prefab(world, Some(living_entity_base))
        // .add(id::<Player>())
        // Hand-written like `EntityFlags` and `Pose`: its wire type is an enum
        // while callers set it from the byte a client settings packet carries.
        .component_and_track::<MainHand>()
        .add_enum(EntityKind::Player)
        .id();

    MetadataPrefabs {
        entity_base,
        arrow_base,
        display_base,
        block_display_base,
        item_base,
        living_entity_base,
        player_base,
    }
}

use super::entity_kind::EntityKind;
use crate::simulation::metadata::r#type::MetadataType;

/// Tracked-value changes accumulated within a gametick.
///
/// The run is built as bytes rather than as values because an entry's length is
/// decided by its serializer, which is also why the terminator belongs to the
/// packet rather than to the run; see [`DataValues`].
#[derive(Debug, Default, Component, Clone)]
pub struct MetadataChanges(DataValues);

unsafe impl Send for MetadataChanges {}

// technically not Sync but I mean do we really care? todo: Indra
unsafe impl Sync for MetadataChanges {}

mod status;

pub mod r#type;

/// One tracked field of an entity: which index it occupies and what it holds.
pub trait Metadata {
    /// The field index, which depends on the entity's class; see the module
    /// documentation for where the numbering comes from.
    const INDEX: u8;
    /// The value this field carries.
    type Type: MetadataType;
    /// The value, ready to write.
    fn to_type(self) -> Self::Type;
}

#[macro_export]
macro_rules! define_metadata_component {
    ($index:literal, $name:ident -> $type:ty) => {
        #[derive(
            Component,
            Clone,
            PartialEq,
            derive_more::Deref,
            derive_more::DerefMut,
            derive_more::Constructor,
            Debug
        )]
        #[allow(clippy::derive_partial_eq_without_eq)]
        pub struct $name {
            value: $type,
        }

        impl Metadata for $name {
            type Type = $type;

            const INDEX: u8 = $index;

            fn to_type(self) -> Self::Type {
                self.value
            }
        }
    };
}

#[macro_export]
macro_rules! register_component_ids {
    ($world:expr, $entity:ident, $($name:ident),* $(,)?) => {
        {
            $(
                let reg = $crate::simulation::metadata::component_and_track::<$name>($world);
                reg(&mut $entity);
            )*

            $entity
        }
    };
}

#[macro_export]
macro_rules! define_and_register_components {
    {
        $(
            $index:literal, $name:ident -> $type:ty
        ),* $(,)?
    } => {
        // Define all components
        $(
            $crate::define_metadata_component!($index, $name -> $type);
        )*

        // Create the registration function
        #[must_use]
        pub fn register_prefab(world: &World, entity_base: Option<Entity>) -> EntityView<'_> {
            // todo: add name
            let mut entity = world.prefab();

            if let Some(entity_base) = entity_base {
                entity = entity.is_a(entity_base);
            }

            $crate::register_component_ids!(
                world,
                entity,
                $($name),*
            )
        }

        /// Encodes every component of this group which is set on `entity` and differs from its
        /// default. Used when a player subscribes to a channel and needs the full current state.
        pub fn encode_non_default_components(
            entity: flecs_ecs::core::EntityView<'_>,
            metadata: &mut $crate::simulation::metadata::MetadataChanges,
        ) {
            use flecs_ecs::core::EntityViewGet;
            $(
                entity.try_get::<&$name>(|component| {
                    metadata.encode_if_not_default(component.clone());
                });
            )*
        }
    };
}

impl MetadataChanges {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn encode_if_not_default<M: Metadata + Default + PartialEq>(&mut self, metadata: M) {
        if metadata == M::default() {
            return;
        }

        self.encode(metadata);
    }

    /// Queue one tracked value for the next [`MetadataChanges`] flush.
    ///
    /// # Panics
    /// Panics when the value does not encode, which for a value this server
    /// built itself means a protocol limit was exceeded.
    pub fn encode<M: Metadata>(&mut self, metadata: M) {
        let value = metadata.to_type();
        self.0
            .push(
                M::INDEX,
                <M::Type as MetadataType>::SERIALIZER,
                &Tracked(&value),
            )
            .expect("a tracked value this server built exceeded a protocol limit");
    }

    /// Encodes the full non-default metadata state of `entity`, so that a player who has just
    /// started observing the entity sees it in its current state rather than its default one.
    pub fn encode_non_default_components(&mut self, entity: EntityView<'_>) {
        let kind = entity
            .try_get::<&EntityKind>(|kind| *kind)
            .expect("entity must have EntityKind component");

        entity.try_get::<&EntityFlags>(|component| {
            self.encode_if_not_default(*component);
        });

        entity.try_get::<&Pose>(|component| {
            self.encode_if_not_default(*component);
        });

        // `MainHand` is registered by hand, so the generated
        // `encode_non_default_components` for the player group does not cover
        // it and a subscriber would otherwise see everyone right-handed.
        if kind == EntityKind::Player {
            entity.try_get::<&MainHand>(|component| {
                self.encode_if_not_default(*component);
            });
        }

        entity::encode_non_default_components(entity, self);

        match kind {
            EntityKind::BlockDisplay => {
                display::encode_non_default_components(entity, self);
                block_display::encode_non_default_components(entity, self);
            }
            EntityKind::Player => {
                living_entity::encode_non_default_components(entity, self);
                player::encode_non_default_components(entity, self);
            }
            EntityKind::Item => {
                item::encode_non_default_components(entity, self);
            }
            // Every kind that reaches `AbstractArrow.tick`, which is the same
            // set `projectile_motion::SIMULATED` gives `MotionOrder::
            // MoveThenDecay`. A subscriber joining after an arrow has already
            // landed has to be told it is in the ground, or its client starts
            // simulating a stopped arrow forwards.
            EntityKind::Arrow | EntityKind::SpectralArrow | EntityKind::Trident => {
                arrow::encode_non_default_components(entity, self);
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct MetadataView<'a>(&'a mut MetadataChanges);

impl core::ops::Deref for MetadataView<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.0.as_bytes()
    }
}

impl Drop for MetadataView<'_> {
    fn drop(&mut self) {
        self.0.0.clear();
    }
}

/// The pending run, or `None` when there is nothing to send.
///
/// The `0xFF` terminator is not here: it belongs to
/// [`hyperion_minecraft_proto::packets::play::entity::SetEntityData`], which
/// writes it after this run. Appending it here as well would send two.
///
/// This is only meant to be called from egress systems.
pub(crate) const fn get_and_clear_metadata(
    metadata: &mut MetadataChanges,
) -> Option<MetadataView<'_>> {
    if metadata.is_empty() {
        return None;
    }
    Some(MetadataView(metadata))
}
