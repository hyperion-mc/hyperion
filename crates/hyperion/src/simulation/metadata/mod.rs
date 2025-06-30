use std::fmt::Debug;

use bevy::prelude::*;
use hyperion_utils::{Prev, track_prev};
use tracing::error;
use valence_protocol::{Encode, VarInt};

use crate::simulation::metadata::entity::{EntityFlags, Pose};

pub mod block_display;
pub mod display;
pub mod entity;
pub mod living_entity;
pub mod player;

/// Set up a system to track metadata changes
fn component_and_track<T>(app: &mut App)
where
    T: Component + Copy + PartialEq + Metadata + Default + Debug,
{
    track_prev::<T>(app);

    // TODO: This will silently ignore changes to the metadata between this system's execution and
    // the time that Prev is updated. There should be a warning for this.
    app.add_systems(
        FixedPostUpdate,
        |mut query: Query<'_, '_, (&Prev<T>, &T, &mut MetadataChanges)>| {
            for (prev, current, mut metadata_changes) in &mut query {
                if **prev != *current {
                    metadata_changes.encode(*current);
                }
            }
        },
    );
}

fn initialize_entity(
    trigger: Trigger<'_, OnInsert, EntityKind>,
    query: Query<'_, '_, &EntityKind>,
    mut commands: Commands<'_, '_>,
) {
    let kind = match query.get(trigger.target()) {
        Ok(kind) => *kind,
        Err(e) => {
            error!("failed to initialize entity: query failed: {e}");
            return;
        }
    };

    let mut entity = commands.entity(trigger.target());

    entity.insert((
        MetadataChanges::default(),
        EntityFlags::default(),
        Pose::default(),
        entity::default_components(),
    ));

    match kind {
        EntityKind::BlockDisplay => {
            entity.insert((
                display::default_components(),
                block_display::default_components(),
            ));
        }
        EntityKind::Player => {
            entity.insert((
                living_entity::default_components(),
                player::default_components(),
            ));
        }
        _ => {}
    }
}

pub struct MetadataPlugin;

impl Plugin for MetadataPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(initialize_entity);
        component_and_track::<EntityFlags>(app);
        component_and_track::<Pose>(app);

        entity::register(app);
        display::register(app);
        block_display::register(app);
        living_entity::register(app);
        player::register(app);
    }
}

use super::entity_kind::EntityKind;
use crate::simulation::metadata::r#type::MetadataType;

#[derive(Debug, Default, Component, Clone)]
// index (u8), type (varint), value (varies)
/// <https://wiki.vg/Entity_metadata>
///
/// Tracks updates within a gametick for the metadata
pub struct MetadataChanges(Vec<u8>);

mod status;

mod r#type;

pub trait Metadata {
    const INDEX: u8;
    type Type: MetadataType + Encode;
    fn to_type(self) -> Self::Type;
}

#[macro_export]
macro_rules! define_metadata_component {
    ($index:literal, $name:ident -> $type:ty) => {
        #[derive(
            Component,
            Copy,
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

        #[allow(warnings)]
        impl PartialOrd for $name
        where
            $type: PartialOrd,
        {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                self.value.partial_cmp(&other.value)
            }
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

        pub fn register(app: &mut App) {
            $(
                $crate::simulation::metadata::component_and_track::<$name>(app);
            )*
        }

        #[must_use]
        pub fn default_components() -> impl bevy::ecs::bundle::Bundle {
            (
                $(
                    $name::default(),
                )*
            )
        }
    };
}

impl MetadataChanges {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn encode<M: Metadata>(&mut self, metadata: M) {
        let value_index = M::INDEX;
        self.0.push(value_index);

        let type_index = VarInt(<M as Metadata>::Type::INDEX);
        type_index.encode(&mut self.0).unwrap();

        let r#type = metadata.to_type();
        r#type.encode(&mut self.0).unwrap();
    }
}

#[derive(Debug)]
pub struct MetadataView<'a>(&'a mut MetadataChanges);

impl core::ops::Deref for MetadataView<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0.0[..]
    }
}

impl Drop for MetadataView<'_> {
    fn drop(&mut self) {
        self.0.0.clear();
    }
}

/// This is only meant to be called from egress systems
pub(crate) fn get_and_clear_metadata(metadata: &mut MetadataChanges) -> Option<MetadataView<'_>> {
    if metadata.is_empty() {
        return None;
    }
    // denote end of metadata
    metadata.0.push(0xff);

    Some(MetadataView(metadata))
}
