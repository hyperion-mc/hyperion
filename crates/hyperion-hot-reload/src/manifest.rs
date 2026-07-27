//! Reads the module tree and the component tree out of a running world.
//!
//! The manifest is taken from a live world rather than generated at build time, so it
//! describes what is actually registered -- including anything a module registered
//! indirectly through a dependency.

use std::collections::{BTreeMap, HashSet};

use flecs_ecs::{
    core::{
        EntityView, EntityViewGet, IdOperations, World, WorldProvider, flecs,
        utility::traits::FlecsConstantId,
    },
    sys,
};

use crate::schema::{ComponentSchema, FieldSchema, Layout};

/// Everything one module put into the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleManifest {
    pub name: String,
    /// Sorted by component symbol so two manifests diff deterministically.
    pub components: Vec<ComponentSchema>,
}

/// The module tree and component tree of a world, as of one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub modules: Vec<ModuleManifest>,
}

impl Manifest {
    #[must_use]
    pub fn module(&self, name: &str) -> Option<&ModuleManifest> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// Replaces one module's entry, keeping the rest untouched.
    pub fn upsert(&mut self, module: ModuleManifest) {
        match self.modules.iter_mut().find(|m| m.name == module.name) {
            Some(slot) => *slot = module,
            None => self.modules.push(module),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.modules.retain(|m| m.name != name);
    }
}

/// The set of entity ids carrying a given tag or component, at one instant.
///
/// Registration deltas are how the engine answers "what did this module register".
/// flecs makes systems, observers and components all entities, so one mechanism covers
/// all three and nothing has to be tracked by hand inside the module.
fn entities_with(world: &World, id: sys::ecs_id_t) -> Vec<sys::ecs_entity_t> {
    let mut out = Vec::new();
    unsafe {
        let mut it = sys::ecs_each_id(world.ptr_mut(), id);
        while sys::ecs_each_next(&raw mut it) {
            let count = usize::try_from(it.count).unwrap_or(0);
            out.extend_from_slice(std::slice::from_raw_parts(it.entities, count));
        }
    }
    out
}

/// Ids of everything a module owns, sampled before and after its registration runs.
#[derive(Clone, Debug, Default)]
pub struct Registrations {
    pub components: Vec<sys::ecs_entity_t>,
    /// Systems and observers hold function pointers into the module's dylib, so these are
    /// exactly what has to be deleted before the dylib stops being the live one.
    pub systems: Vec<sys::ecs_entity_t>,
    pub observers: Vec<sys::ecs_entity_t>,
    /// Entities carrying the `flecs::Module` tag.
    ///
    /// `world.import::<M>()` is idempotent: it short-circuits when the module entity is
    /// already tagged, so without clearing the tag a reload would register nothing and the
    /// old code would keep running while reporting success. The tag is removed rather than
    /// the entity deleted, because a module's components are registered as its children
    /// and deleting it would take their stored data with it.
    pub modules: Vec<sys::ecs_entity_t>,
}

/// A sample of the world's registrations, used as the "before" half of a delta.
pub struct WorldSample {
    components: Vec<sys::ecs_entity_t>,
    systems: Vec<sys::ecs_entity_t>,
    observers: Vec<sys::ecs_entity_t>,
    modules: Vec<sys::ecs_entity_t>,
}

impl WorldSample {
    #[must_use]
    pub fn take(world: &World) -> Self {
        Self {
            components: entities_with(world, <flecs::Component as FlecsConstantId>::ID),
            systems: entities_with(world, <flecs::system::System as FlecsConstantId>::ID),
            observers: entities_with(world, <flecs::Observer as FlecsConstantId>::ID),
            modules: entities_with(world, <flecs::Module as FlecsConstantId>::ID),
        }
    }

    /// What appeared in the world since this sample was taken.
    #[must_use]
    pub fn delta(&self, world: &World) -> Registrations {
        let now = Self::take(world);
        Registrations {
            components: added(&self.components, &now.components),
            systems: added(&self.systems, &now.systems),
            observers: added(&self.observers, &now.observers),
            modules: added(&self.modules, &now.modules),
        }
    }
}

fn added(before: &[sys::ecs_entity_t], after: &[sys::ecs_entity_t]) -> Vec<sys::ecs_entity_t> {
    let before: HashSet<_> = before.iter().copied().collect();
    let mut out: Vec<_> = after.iter().copied().filter(|e| !before.contains(e)).collect();
    out.sort_unstable();
    out
}

/// Whether an entity carries one of flecs' meta type-kind components.
fn has_kind(entity: EntityView<'_>, kind: sys::ecs_entity_t) -> bool {
    unsafe { sys::ecs_has_id(entity.world().ptr_mut(), *entity.id(), kind) }
}

/// The members of a struct type, in offset order.
fn members_of(ty: EntityView<'_>) -> Vec<(String, sys::ecs_entity_t, i32, i32)> {
    let mut out = Vec::new();
    ty.each_child(|child| {
        if !child.has(<flecs::meta::Member as FlecsConstantId>::ID) {
            return;
        }
        child.get::<&flecs::meta::Member>(|m| {
            out.push((child.name().to_string(), m.type_, m.offset, m.count));
        });
    });
    // flecs stores members as child entities and does not promise an order.
    out.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    out
}

/// The constants of an enum type, in declaration order.
///
/// flecs creates one child entity per constant and allocates them in declaration order,
/// so sorting by entity id recovers that order.
fn constants_of(ty: EntityView<'_>) -> Vec<String> {
    let mut out: Vec<(u64, String)> = Vec::new();
    ty.each_child(|child| {
        if child.has(<flecs::meta::Member as FlecsConstantId>::ID) {
            return;
        }
        out.push((*child.id(), child.name().to_string()));
    });
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.into_iter().map(|(_, name)| name).collect()
}

/// How deep a component's field tree may nest before the gate gives up and refuses.
///
/// A cap rather than a cycle set because flecs meta cannot express a recursive value type
/// -- a struct containing itself has no finite size -- so any depth this large means
/// something the gate does not understand.
const MAX_DEPTH: u32 = 16;

/// Flattens one type's primitive leaves into `out` at absolute offsets.
///
/// Returns `false` when any leaf could not be reduced to something the gate can compare,
/// which makes the whole component unprovable rather than partly checked. A partly
/// checked component is the dangerous case: it compares equal while the unchecked part
/// changed underneath.
fn flatten(
    ty: EntityView<'_>,
    prefix: &str,
    base: i32,
    depth: u32,
    out: &mut Vec<FieldSchema>,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let size = if ty.has(<flecs::Component as FlecsConstantId>::ID) {
        ty.get::<&flecs::Component>(|c| c.size)
    } else {
        0
    };

    // A primitive is the only thing that terminates the walk successfully.
    if has_kind(ty, unsafe { sys::FLECS_IDEcsPrimitiveID_ }) {
        out.push(FieldSchema {
            name: prefix.to_owned(),
            type_name: ty.symbol().to_string(),
            offset: base,
            size,
            count: 0,
        });
        return true;
    }

    // A nested enum becomes one leaf whose type string carries its constants, so adding,
    // removing, renaming or reordering a variant changes the enclosing component's hash.
    if has_kind(ty, unsafe { sys::FLECS_IDEcsEnumID_ }) {
        let constants = constants_of(ty);
        out.push(FieldSchema {
            name: prefix.to_owned(),
            type_name: format!("enum({})", constants.join("|")),
            offset: base,
            size,
            count: 0,
        });
        return true;
    }

    if has_kind(ty, unsafe { sys::FLECS_IDEcsStructID_ }) {
        let members = members_of(ty);
        if members.is_empty() {
            // A struct kind with no members is either genuinely empty or something whose
            // members flecs could not describe. Neither is safe to call "unchanged".
            return false;
        }
        for (name, member_ty, offset, count) in members {
            // An inline array member repeats its element type; the gate has no way to see
            // past the first element, so refuse rather than check one element and imply
            // the rest.
            if count > 1 {
                return false;
            }
            let child_prefix = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}.{name}")
            };
            let child_ty = EntityView::new_from(ty.world(), member_ty);
            if !flatten(child_ty, &child_prefix, base + offset, depth + 1, out) {
                return false;
            }
        }
        return true;
    }

    // Arrays, vectors, bitmasks and opaque types all describe storage the gate cannot walk.
    false
}

/// Reads one component entity's schema.
///
/// `opaque_versions` supplies the hand-maintained version for components that carry no
/// reflection; anything absent from it stays [`Layout::Unknown`] and the gate will refuse
/// to carry its data across a reload.
#[must_use]
pub fn read_component_schema(
    component: EntityView<'_>,
    opaque_versions: &BTreeMap<String, u32>,
) -> ComponentSchema {
    // The symbol, not the path: importing a module re-parents its components underneath
    // the module entity, so the path is not stable identity. See `ComponentSchema::name`.
    let name = {
        let symbol = component.symbol().to_string();
        if symbol.is_empty() {
            component
                .path()
                .unwrap_or_default()
                .strip_prefix("::")
                .unwrap_or_default()
                .to_owned()
        } else {
            symbol
        }
    };
    let (size, alignment) = component.get::<&flecs::Component>(|c| (c.size, c.alignment));

    let layout = if has_kind(component, unsafe { sys::FLECS_IDEcsEnumID_ }) {
        Layout::Enum(constants_of(component))
    } else {
        let mut fields = Vec::new();
        if flatten(component, "", 0, 0, &mut fields) && !fields.is_empty() {
            Layout::Reflected(fields)
        } else {
            opaque_versions
                .get(&name)
                .map_or(Layout::Unknown, |&v| Layout::Opaque {
                    declared_version: v,
                })
        }
    };

    ComponentSchema {
        name,
        size,
        alignment,
        layout,
    }
}

/// Builds a module's manifest entry from the components its registration created.
#[must_use]
pub fn describe_module(
    world: &World,
    name: &str,
    registrations: &Registrations,
    opaque_versions: &BTreeMap<String, u32>,
) -> ModuleManifest {
    let mut components: Vec<ComponentSchema> = registrations
        .components
        .iter()
        .map(|&id| read_component_schema(world.entity_from_id(id), opaque_versions))
        .collect();
    components.sort_by(|a, b| a.name.cmp(&b.name));
    ModuleManifest {
        name: name.to_owned(),
        components,
    }
}
