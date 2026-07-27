//! What the schema reader actually sees in a live flecs world.
//!
//! These tests exist because the dangerous failure is not a refusal, it is a component
//! whose layout changed while its schema compared equal. Each one pins a case where a
//! shallower reader would have said "unchanged".

use std::collections::BTreeMap;

use flecs_ecs::{core::World, prelude::*};
use hyperion_hot_reload::{ComponentSchema, Layout, read_component_schema};

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
struct InnerTwoShorts {
    a: u16,
    b: u16,
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
struct InnerByteByteShort {
    a: u8,
    b: u8,
    c: u16,
}

/// Same size and alignment as [`OuterB`], and its top-level members have identical names,
/// offsets and sizes. Only the interior of the first member differs.
#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
struct OuterA {
    inner: InnerTwoShorts,
    tail: u32,
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
struct OuterB {
    inner: InnerByteByteShort,
    tail: u32,
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
#[repr(C)]
enum ModeForward {
    #[default]
    Idle,
    Busy,
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
#[repr(C)]
enum ModeReversed {
    #[default]
    Busy,
    Idle,
}

/// No `#[flecs(meta)]`, and a field flecs could not describe even if it had one.
#[derive(Component, Debug, Default)]
struct Unreflected {
    _name: String,
}

fn schema_of<T: ComponentId + DataComponent>(
    world: &World,
    opaque: &BTreeMap<String, u32>,
) -> ComponentSchema {
    read_component_schema(world.component::<T>().entity_view(world), opaque)
}

const fn no_opaque() -> BTreeMap<String, u32> {
    BTreeMap::new()
}

#[test]
fn nested_struct_interiors_are_flattened_to_primitive_leaves() {
    let world = World::new();
    world.component::<InnerTwoShorts>().meta();
    world.component::<OuterA>().meta();

    let schema = schema_of::<OuterA>(&world, &no_opaque());
    let Layout::Reflected(fields) = &schema.layout else {
        panic!("expected a reflected layout, got {:?}", schema.layout);
    };

    let rendered: Vec<String> = fields
        .iter()
        .map(|f| format!("{}:{}@{}", f.name, f.type_name, f.offset))
        .collect();
    assert_eq!(
        rendered,
        vec!["inner.a:u16@0", "inner.b:u16@2", "tail:u32@4"],
        "the nested member must be expanded, not recorded as a type name"
    );
}

/// The hole this closes: two components whose top-level member lists are byte-for-byte
/// identical, differing only inside a nested type. A reader that records `inner` as a
/// type name calls these equal and reinterprets the bytes.
#[test]
fn a_nested_interior_change_changes_the_schema_hash() {
    let world = World::new();
    world.component::<InnerTwoShorts>().meta();
    world.component::<InnerByteByteShort>().meta();
    world.component::<OuterA>().meta();
    world.component::<OuterB>().meta();

    let a = schema_of::<OuterA>(&world, &no_opaque());
    let b = schema_of::<OuterB>(&world, &no_opaque());

    // Everything a shallow reader would compare is equal.
    assert_eq!(a.size, b.size, "same size");
    assert_eq!(a.alignment, b.alignment, "same alignment");
    let (Layout::Reflected(fa), Layout::Reflected(fb)) = (&a.layout, &b.layout) else {
        panic!("both must reflect");
    };
    let shallow = |fields: &[hyperion_hot_reload::FieldSchema]| -> Vec<(i32, i32)> {
        fields.iter().map(|f| (f.offset, f.size)).collect()
    };
    assert_ne!(
        shallow(fa),
        shallow(fb),
        "flattening must produce different leaves"
    );

    assert_ne!(
        a.hash(),
        b.hash(),
        "a change inside a nested member must change the hash"
    );
    assert!(
        !a.is_bit_compatible_with(&b),
        "a nested interior change must not be bit compatible"
    );
}

#[test]
fn enum_variant_order_is_part_of_the_schema() {
    let world = World::new();
    world.component::<ModeForward>().meta();
    world.component::<ModeReversed>().meta();

    let forward = schema_of::<ModeForward>(&world, &no_opaque());
    let reversed = schema_of::<ModeReversed>(&world, &no_opaque());

    assert_eq!(
        forward.layout,
        Layout::Enum(vec!["Idle".to_owned(), "Busy".to_owned()])
    );
    assert_eq!(
        reversed.layout,
        Layout::Enum(vec!["Busy".to_owned(), "Idle".to_owned()])
    );
    assert_eq!(forward.size, reversed.size, "same size");
    assert!(
        !forward.is_bit_compatible_with(&reversed),
        "reordering variants changes what a stored discriminant means"
    );
}

/// A component flecs cannot describe must land in `Unknown`, never in `Reflected` with an
/// empty or partial field list.
#[test]
fn a_component_without_reflection_is_unknown() {
    let world = World::new();
    world.component::<Unreflected>();
    let schema = schema_of::<Unreflected>(&world, &no_opaque());
    assert_eq!(schema.layout, Layout::Unknown);
    assert!(
        !schema.is_bit_compatible_with(&schema.clone()),
        "an unknown interior is never provably unchanged, even against itself"
    );
}

#[test]
fn a_declared_opaque_version_replaces_unknown() {
    let world = World::new();
    world.component::<Unreflected>();
    let mut opaque = BTreeMap::new();
    opaque.insert(core::any::type_name::<Unreflected>().to_owned(), 7);
    let schema = schema_of::<Unreflected>(&world, &opaque);
    assert_eq!(schema.layout, Layout::Opaque {
        declared_version: 7
    });
    assert!(
        schema.is_bit_compatible_with(&schema.clone()),
        "a vouched-for component at the same version is unchanged"
    );
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
struct ScopedHealth {
    hp: u32,
}

#[derive(Component)]
struct ScopingModule;

impl Module for ScopingModule {
    fn module(world: &World) {
        world.component::<ScopedHealth>().meta();
    }
}

/// Importing a module re-parents its components underneath the module entity, so the path
/// gains a segment the type name never had. Identity has to survive that.
#[test]
fn component_identity_is_the_symbol_not_the_path() {
    let world = World::new();
    world.import::<ScopingModule>();

    let view = world.component::<ScopedHealth>().entity_view(&world);
    let path = view.path().unwrap_or_default();
    assert!(
        path.contains("ScopingModule"),
        "expected the module to scope the component, got {path}"
    );

    let schema = read_component_schema(view, &no_opaque());
    assert_eq!(schema.name, core::any::type_name::<ScopedHealth>());
    assert!(
        !schema.name.contains("ScopingModule"),
        "identity must not carry the module scope, got {}",
        schema.name
    );
}
