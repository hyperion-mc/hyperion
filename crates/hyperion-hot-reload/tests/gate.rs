//! What the gate accepts, and what it refuses.
//!
//! These are pure schema comparisons: no world, no dylib. The point is to pin the
//! decisions themselves, so a refactor of the reflection code cannot quietly turn a
//! refusal into an acceptance.

use hyperion_hot_reload::{
    ComponentSchema, FieldSchema, Layout, Migration, ModuleManifest, Refusal, gate,
};

fn field(name: &str, ty: &str, offset: i32, size: i32) -> FieldSchema {
    FieldSchema {
        name: name.to_owned(),
        type_name: ty.to_owned(),
        offset,
        size,
        count: 0,
    }
}

fn health(ty: &str) -> ComponentSchema {
    ComponentSchema {
        name: "demo::Health".to_owned(),
        size: 4,
        alignment: 4,
        layout: Layout::Reflected(vec![field("hp", ty, 0, 4)]),
    }
}

fn module(components: Vec<ComponentSchema>) -> ModuleManifest {
    ModuleManifest {
        name: "arena".to_owned(),
        components,
    }
}

/// Stands in for a world that holds data for every component.
fn has_data(_: &str) -> bool {
    true
}

fn no_data(_: &str) -> bool {
    false
}

fn noop_migration(_: &[u8], _: &mut [u8]) {}

#[test]
fn identical_schemas_keep_their_data() {
    let old = module(vec![health("u32")]);
    let new = module(vec![health("u32")]);
    let plan = gate::plan(Some(&old), &new, &[], &has_data).expect("identical schema must pass");
    assert_eq!(plan.kept, vec!["demo::Health"]);
    assert!(plan.migrations.is_empty());
}

/// The central guard: a changed layout with no migration must never be waved through.
#[test]
fn refuses_a_layout_change_with_no_migration() {
    let old = module(vec![health("u32")]);
    let new = module(vec![health("f32")]);

    let refused = gate::plan(Some(&old), &new, &[], &has_data)
        .expect_err("u32 -> f32 with no migration must be refused");

    assert_eq!(refused.reasons.len(), 1);
    let Refusal::MissingMigration { old, new } = &refused.reasons[0] else {
        panic!("expected MissingMigration, got {:?}", refused.reasons[0]);
    };
    assert_eq!(old.name, "demo::Health");
    assert_ne!(old.hash(), new.hash());

    // The message has to name the component and both layouts, because that is the whole
    // value of refusing rather than corrupting.
    let rendered = refused.to_string();
    assert!(rendered.contains("demo::Health"), "{rendered}");
    assert!(rendered.contains("hp: u32"), "{rendered}");
    assert!(rendered.contains("hp: f32"), "{rendered}");
    assert!(rendered.contains("migration!"), "{rendered}");
    assert!(
        rendered.contains("The running world was not modified"),
        "{rendered}"
    );
}

#[test]
fn accepts_a_layout_change_when_a_matching_migration_exists() {
    let old = module(vec![health("u32")]);
    let new = module(vec![health("f32")]);
    let migrations = vec![Migration {
        component: "demo::Health".to_owned(),
        from: health("u32"),
        apply: noop_migration,
    }];

    let plan = gate::plan(Some(&old), &new, &migrations, &has_data)
        .expect("a matching migration must be accepted");
    assert_eq!(plan.migrations.len(), 1);
    assert_eq!(plan.migrations[0].component, "demo::Health");
    assert_eq!(plan.migrations[0].migration_index, 0);
}

/// A migration that describes an old layout the world does not have would read the live
/// bytes at the wrong offsets. That is worse than no migration at all.
#[test]
fn refuses_a_migration_that_declares_the_wrong_old_layout() {
    let old = module(vec![health("u32")]);
    let new = module(vec![health("f32")]);
    let mut wrong = health("u32");
    wrong.size = 8;
    let migrations = vec![Migration {
        component: "demo::Health".to_owned(),
        from: wrong,
        apply: noop_migration,
    }];

    let refused = gate::plan(Some(&old), &new, &migrations, &has_data)
        .expect_err("a migration declaring the wrong old layout must be refused");
    assert!(matches!(
        refused.reasons[0],
        Refusal::MigrationDoesNotMatch { .. }
    ));
}

#[test]
fn refuses_an_unprovable_layout_that_holds_data() {
    let opaque = |v: Layout| ComponentSchema {
        name: "demo::Blob".to_owned(),
        size: 24,
        alignment: 8,
        layout: v,
    };
    let old = module(vec![opaque(Layout::Unknown)]);
    let new = module(vec![opaque(Layout::Unknown)]);

    let refused = gate::plan(Some(&old), &new, &[], &has_data)
        .expect_err("an unreflected component holding data must be refused");
    assert!(matches!(
        refused.reasons[0],
        Refusal::UnprovableLayout { .. }
    ));
}

/// The same component with nothing stored in it cannot be corrupted, so it is not a
/// reason to refuse a reload.
#[test]
fn allows_an_unprovable_layout_with_no_stored_data() {
    let opaque = ComponentSchema {
        name: "demo::Blob".to_owned(),
        size: 24,
        alignment: 8,
        layout: Layout::Unknown,
    };
    let old = module(vec![opaque.clone()]);
    let new = module(vec![opaque]);
    let plan = gate::plan(Some(&old), &new, &[], &no_data).expect("no stored data, nothing to lose");
    assert_eq!(plan.kept, vec!["demo::Blob"]);
}

/// A developer who bumps the declared version is telling the gate the interior changed.
#[test]
fn an_opaque_version_bump_is_a_layout_change() {
    let at = |v: u32| ComponentSchema {
        name: "demo::Blob".to_owned(),
        size: 24,
        alignment: 8,
        layout: Layout::Opaque {
            declared_version: v,
        },
    };
    let same = gate::plan(Some(&module(vec![at(1)])), &module(vec![at(1)]), &[], &has_data);
    assert!(same.is_ok(), "an unchanged version must be accepted");

    let bumped = gate::plan(Some(&module(vec![at(1)])), &module(vec![at(2)]), &[], &has_data);
    assert!(
        bumped.is_err(),
        "a bumped version with no migration must be refused"
    );
}

/// A tag has no bytes per entity, so growing one into a real component cannot
/// misinterpret anything that was already stored.
#[test]
fn a_tag_that_grows_fields_is_not_refused() {
    let tag = ComponentSchema {
        name: "demo::Frozen".to_owned(),
        size: 0,
        alignment: 0,
        layout: Layout::Unknown,
    };
    let grown = ComponentSchema {
        name: "demo::Frozen".to_owned(),
        size: 4,
        alignment: 4,
        layout: Layout::Reflected(vec![field("ticks", "u32", 0, 4)]),
    };
    let plan = gate::plan(Some(&module(vec![tag])), &module(vec![grown]), &[], &has_data)
        .expect("a tag has no stored bytes to misinterpret");
    assert_eq!(plan.kept, vec!["demo::Frozen"]);
}

#[test]
fn added_and_dropped_components_are_reported() {
    let old = module(vec![health("u32")]);
    let new = module(vec![ComponentSchema {
        name: "demo::Mana".to_owned(),
        size: 4,
        alignment: 4,
        layout: Layout::Reflected(vec![field("mp", "u32", 0, 4)]),
    }]);
    let plan = gate::plan(Some(&old), &new, &[], &has_data).expect("add and drop are not changes");
    assert_eq!(plan.added, vec!["demo::Mana"]);
    assert_eq!(plan.dropped, vec!["demo::Health"]);
}
