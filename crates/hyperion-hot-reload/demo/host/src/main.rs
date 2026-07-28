//! Drives a live world through a sequence of module builds and reports what the gate
//! decided for each one.
//!
//! Usage: `hot-reload-demo <module.dylib> [<module.dylib> ...]`
//!
//! The world, its entities and their component data are created once, before the first
//! reload, and never rebuilt. Anything still present at the end survived every reload.
#![allow(
    clippy::print_stdout,
    reason = "this binary's whole purpose is printing a reload transcript"
)]

use std::path::Path;

use flecs_ecs::{
    core::{Entity, EntityView, IdOperations, World},
    sys,
};
use hyperion_hot_reload::{HotReloader, Layout, LoadError, ModuleManifest, lookup_component};

/// Prints a heading that is easy to find in a transcript.
fn banner(text: &str) {
    println!("\n=== {text} ===");
}

/// Writes a component's bytes directly, the way a save-game loader would.
fn set_raw(world: &World, entity: Entity, component: &str, bytes: &[u8]) {
    let id = lookup_component(world, component)
        .unwrap_or_else(|| panic!("component `{component}` is not registered"));
    unsafe {
        let ptr = sys::ecs_ensure_id(world.ptr_mut(), *entity, id, bytes.len());
        assert!(!ptr.is_null(), "could not store `{component}`");
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
    }
}

/// Renders a component's stored bytes using the schema the manifest currently reports.
fn render(world: &World, entity: EntityView<'_>, manifest: &ModuleManifest, name: &str) -> String {
    let Some(schema) = manifest.components.iter().find(|c| c.name == name) else {
        return "<not in manifest>".to_owned();
    };
    let Some(bytes) = hyperion_hot_reload::read_raw(world, entity, name) else {
        return "<absent>".to_owned();
    };
    let Layout::Reflected(fields) = &schema.layout else {
        return format!("{bytes:02x?}");
    };
    let rendered: Vec<String> = fields
        .iter()
        .map(|f| {
            let start = usize::try_from(f.offset).unwrap_or(0);
            let end = start + usize::try_from(f.size).unwrap_or(0);
            let raw = &bytes[start..end];
            let value = match f.type_name.as_str() {
                "u32" => u32::from_le_bytes(raw.try_into().unwrap_or([0; 4])).to_string(),
                "i32" => i32::from_le_bytes(raw.try_into().unwrap_or([0; 4])).to_string(),
                "f32" => {
                    format!(
                        "{:.1}",
                        f32::from_le_bytes(raw.try_into().unwrap_or([0; 4]))
                    )
                }
                other => format!("<{other} {raw:02x?}>"),
            };
            format!("{}: {} = {value}", f.name, f.type_name)
        })
        .collect();
    rendered.join(", ")
}

fn dump(world: &World, reloader: &HotReloader, entities: &[Entity]) {
    let manifest = reloader.manifest();
    let arena = manifest.module("arena").expect("arena module");
    println!("  manifest:");
    for component in &arena.components {
        println!("    {}", component.describe());
    }
    for (i, &entity) in entities.iter().enumerate() {
        let view = world.entity_from_id(*entity);
        println!(
            "  entity {i}: {} | {}",
            render(
                world,
                view,
                arena,
                "hyperion_hot_reload_demo_module::Health"
            ),
            render(world, view, arena, "hyperion_hot_reload_demo_module::Score")
        );
    }
}

fn main() {
    let builds: Vec<String> = std::env::args().skip(1).collect();
    assert!(
        !builds.is_empty(),
        "usage: hot-reload-demo <module.dylib>..."
    );

    let world = World::new();
    let mut reloader = HotReloader::new();

    banner(&format!("initial load: {}", builds[0]));
    let first = reloader
        .load(&world, Path::new(&builds[0]))
        .unwrap_or_else(|e| panic!("initial load failed: {e}"));
    println!("  loaded module `{}`", first.module);
    for c in &reloader
        .manifest()
        .module("arena")
        .expect("arena")
        .components
    {
        println!("    registered: {}", c.describe());
    }

    // World state is created once, here, and never touched again by the demo.
    let entities: Vec<Entity> = (0..3)
        .map(|i| {
            let e = world.entity();
            set_raw(
                &world,
                e.id(),
                "hyperion_hot_reload_demo_module::Health",
                &(10u32 * (i + 1)).to_le_bytes(),
            );
            set_raw(
                &world,
                e.id(),
                "hyperion_hot_reload_demo_module::Score",
                &(7i32 * i32::try_from(i + 1).unwrap_or(1)).to_le_bytes(),
            );
            e.id()
        })
        .collect();

    println!("  spawned {} entities", entities.len());
    world.progress();
    println!("  after one tick:");
    dump(&world, &reloader, &entities);

    for build in &builds[1..] {
        banner(&format!("reload: {build}"));
        match reloader.load(&world, Path::new(build)) {
            Ok(applied) => {
                println!("  ACCEPTED");
                println!(
                    "    kept: {:?}\n    added: {:?}\n    dropped: {:?}",
                    applied.plan.kept, applied.plan.added, applied.plan.dropped
                );
                for m in &applied.plan.migrations {
                    println!(
                        "    migrated `{}`: {} -> {}",
                        m.component,
                        m.old.hash(),
                        m.new.hash()
                    );
                }
                println!("    instances rewritten: {}", applied.migrated_instances);
                world.progress();
                println!("  after one tick:");
                dump(&world, &reloader, &entities);
            }
            Err(LoadError::Refused(refused)) => {
                println!("  REFUSED\n{refused}");
                world.progress();
                println!("  world still running on the previous build:");
                dump(&world, &reloader, &entities);
            }
            Err(other) => println!("  ERROR: {other}"),
        }
    }
}
