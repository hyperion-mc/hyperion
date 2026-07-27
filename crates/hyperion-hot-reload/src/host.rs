//! The host side: load a candidate build, gate it, and only then change the live world.
//!
//! Nothing in the live world is touched until [`gate::plan`] has accepted the candidate,
//! which is why a refused reload leaves a running server exactly as it was.

use std::{collections::BTreeMap, path::Path};

use flecs_ecs::{
    core::{Entity, EntityView, EntityViewGet, IdOperations, World, flecs, utility::traits::FlecsConstantId},
    sys,
};

use crate::{
    abi::{ENTRY_SYMBOL, Migration, ModuleDescriptor, ModuleEntry},
    gate::{self, Plan, Refused},
    manifest::{Manifest, ModuleManifest, Registrations, WorldSample, describe_module},
};

/// What a module currently occupies in the live world.
struct Loaded {
    manifest: ModuleManifest,
    /// Systems and observers hold function pointers into the module's dylib; they are
    /// deleted and re-created on every reload, whether or not any schema changed.
    registrations: Registrations,
}

/// Why a load could not even be attempted.
#[derive(Debug)]
pub enum LoadError {
    Dlopen(libloading::Error),
    MissingEntry(libloading::Error),
    /// The module and the host disagree about the runtime they share.
    Abi(String),
    Refused(Refused),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Dlopen(e) => write!(f, "could not open module: {e}"),
            Self::MissingEntry(e) => {
                write!(f, "module exports no `hyperion_hot_module` entry point: {e}")
            }
            Self::Abi(m) => write!(f, "refusing to load module: {m}"),
            Self::Refused(r) => write!(f, "{r}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// A successful load.
#[derive(Debug)]
pub struct Applied {
    pub module: String,
    pub plan: Plan,
    /// Number of stored component instances rewritten by migrations.
    pub migrated_instances: usize,
}

/// Owns the live world's hot-reloadable modules.
pub struct HotReloader {
    loaded: BTreeMap<String, Loaded>,
    /// Every dylib ever loaded, deliberately never closed.
    ///
    /// flecs stores `dtor`/`move_dtor` function pointers into the module's text segment in
    /// each component's type info, and there is no API to unset them. `dlclose` is in any
    /// case a no-op on macOS and a deferred leak on glibc once a Rust dylib has registered
    /// a thread-local destructor. Keeping the mapping alive turns a latent jump-into-
    /// unmapped-memory into bounded address-space growth.
    retained: Vec<libloading::Library>,
}

impl Default for HotReloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HotReloader {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            loaded: BTreeMap::new(),
            retained: Vec::new(),
        }
    }

    /// The module tree and component tree currently live.
    #[must_use]
    pub fn manifest(&self) -> Manifest {
        Manifest {
            modules: self.loaded.values().map(|l| l.manifest.clone()).collect(),
        }
    }

    /// Loads a build of a module, replacing any previous build of the same name.
    ///
    /// # Errors
    /// Returns [`LoadError::Refused`] when a component's layout changed without a
    /// migration; in that case the world is untouched.
    ///
    /// # Panics
    /// Panics if the module's registration panics.
    pub fn load(&mut self, world: &World, path: &Path) -> Result<Applied, LoadError> {
        let lib = unsafe { libloading::Library::new(path) }.map_err(LoadError::Dlopen)?;
        let descriptor = {
            let entry: libloading::Symbol<'_, ModuleEntry> =
                unsafe { lib.get(ENTRY_SYMBOL) }.map_err(LoadError::MissingEntry)?;
            let raw = unsafe { entry() };
            *unsafe { Box::from_raw(raw) }
        };
        // Retained before any of its code runs, so an early return still leaves the
        // mapping alive for whatever already holds a pointer into it.
        self.retained.push(lib);

        if let Some(reason) = descriptor.token.incompatibility() {
            return Err(LoadError::Abi(reason));
        }

        let opaque: BTreeMap<String, u32> =
            descriptor.opaque_versions.iter().cloned().collect();

        // Register into a throwaway world first. The candidate's schema is only knowable
        // by running its registration, and running it in the live world is exactly what
        // the gate exists to authorise.
        let candidate = {
            let probe = World::new();
            let before = WorldSample::take(&probe);
            (descriptor.register)(&probe);
            let regs = before.delta(&probe);
            describe_module(&probe, &descriptor.name, &regs, &opaque)
        };

        let plan = gate::plan(
            self.loaded.get(&descriptor.name).map(|l| &l.manifest),
            &candidate,
            &descriptor.migrations,
            &|name| component_instance_count(world, name) > 0,
        )
        .map_err(LoadError::Refused)?;

        let migrated = self.apply(world, &descriptor, &candidate, &plan);

        Ok(Applied {
            module: descriptor.name,
            plan,
            migrated_instances: migrated,
        })
    }

    /// Everything past the gate. Ordered so that no step can observe a half-updated world:
    /// harvest, tear down, register, then write the migrated bytes back.
    fn apply(
        &mut self,
        world: &World,
        descriptor: &ModuleDescriptor,
        candidate: &ModuleManifest,
        plan: &Plan,
    ) -> usize {
        let mut harvested: Vec<(&crate::gate::PlannedMigration, Vec<(Entity, Vec<u8>)>)> =
            Vec::new();
        for planned in &plan.migrations {
            let Some(id) = lookup_component(world, &planned.component) else {
                continue;
            };
            let rows = harvest(world, id, usize::try_from(planned.old.size).unwrap_or(0));
            harvested.push((planned, rows));
        }

        let mut module_entities = Vec::new();
        if let Some(previous) = self.loaded.remove(&descriptor.name) {
            // Systems and observers first: they are the live function pointers into the
            // build being replaced, and leaving them would run the old and the new code
            // side by side.
            for id in previous
                .registrations
                .systems
                .iter()
                .chain(&previous.registrations.observers)
            {
                unsafe { sys::ecs_delete(world.ptr_mut(), *id) };
            }
            module_entities = previous.registrations.modules;
            for id in &module_entities {
                unsafe {
                    sys::ecs_remove_id(
                        world.ptr_mut(),
                        *id,
                        <flecs::Module as FlecsConstantId>::ID,
                    );
                };
            }
        }

        // A component whose size changed cannot be re-registered over the old entity:
        // flecs aborts the process with ECS_INVALID_COMPONENT_SIZE. Delete it so the new
        // build registers a fresh one.
        for planned in &plan.migrations {
            if let Some(id) = lookup_component(world, &planned.component) {
                unsafe { sys::ecs_delete(world.ptr_mut(), id) };
            }
        }
        for name in &plan.dropped {
            if let Some(id) = lookup_component(world, name) {
                unsafe { sys::ecs_delete(world.ptr_mut(), id) };
            }
        }

        let before = WorldSample::take(world);
        (descriptor.register)(world);
        let mut registrations = before.delta(world);
        // Carried forward: after the first load the module entity already exists, so it
        // never shows up in a later delta, yet its tag still has to be cleared next time.
        for id in module_entities {
            if !registrations.modules.contains(&id) {
                registrations.modules.push(id);
            }
        }

        let mut migrated = 0;
        for (planned, rows) in harvested {
            let Some(new_id) = lookup_component(world, &planned.component) else {
                continue;
            };
            let migration = &descriptor.migrations[planned.migration_index];
            migrated += write_migrated(world, new_id, &planned.new, migration, &rows);
        }

        self.loaded.insert(
            descriptor.name.clone(),
            Loaded {
                manifest: candidate.clone(),
                registrations,
            },
        );
        migrated
    }
}

/// Finds a component entity by its flecs symbol.
///
/// By symbol rather than by path: `world.import::<M>()` scopes a module's components
/// underneath the module entity, so `Health` is at `::demo::ArenaModule::Health` while its
/// symbol stays `demo::Health`. The symbol is what survives a module being renamed, and
/// what the manifest keys on.
#[must_use]
pub fn lookup_component(world: &World, symbol: &str) -> Option<sys::ecs_entity_t> {
    let Ok(c) = std::ffi::CString::new(symbol) else {
        return None;
    };
    let id = unsafe { sys::ecs_lookup_symbol(world.ptr_mut(), c.as_ptr(), true, true) };
    (id != 0).then_some(id)
}

/// How many entities currently store this component.
#[must_use]
pub fn component_instance_count(world: &World, name: &str) -> usize {
    let Some(id) = lookup_component(world, name) else {
        return 0;
    };
    let mut total = 0;
    unsafe {
        let mut it = sys::ecs_each_id(world.ptr_mut(), id);
        while sys::ecs_each_next(&raw mut it) {
            total += usize::try_from(it.count).unwrap_or(0);
        }
    }
    total
}

/// Copies every stored instance's raw bytes out before the component entity is deleted.
///
/// Reading raw bytes is sound here only because the gate has already checked that a
/// migration declared exactly this layout, and the size below is that declared size.
fn harvest(world: &World, id: sys::ecs_entity_t, size: usize) -> Vec<(Entity, Vec<u8>)> {
    let mut out = Vec::new();
    if size == 0 {
        return out;
    }
    unsafe {
        let mut it = sys::ecs_each_id(world.ptr_mut(), id);
        while sys::ecs_each_next(&raw mut it) {
            let count = usize::try_from(it.count).unwrap_or(0);
            let base = sys::ecs_field_w_size(&raw const it, size, 0).cast::<u8>();
            if base.is_null() {
                continue;
            }
            let entities = std::slice::from_raw_parts(it.entities, count);
            for (row, &entity) in entities.iter().enumerate() {
                let src = std::slice::from_raw_parts(base.add(row * size), size);
                out.push((Entity::new(entity), src.to_vec()));
            }
        }
    }
    out
}

/// Runs the developer's migration for each harvested instance and stores the result.
fn write_migrated(
    world: &World,
    new_id: sys::ecs_entity_t,
    new_schema: &crate::schema::ComponentSchema,
    migration: &Migration,
    rows: &[(Entity, Vec<u8>)],
) -> usize {
    let new_size = usize::try_from(new_schema.size).unwrap_or(0);
    if new_size == 0 {
        return 0;
    }
    let mut buf = vec![0u8; new_size];
    let mut done = 0;
    for (entity, old_bytes) in rows {
        buf.iter_mut().for_each(|b| *b = 0);
        (migration.apply)(old_bytes, &mut buf);
        unsafe {
            let dst = sys::ecs_ensure_id(world.ptr_mut(), **entity, new_id, new_size);
            if dst.is_null() {
                continue;
            }
            std::ptr::copy_nonoverlapping(buf.as_ptr(), dst.cast::<u8>(), new_size);
        }
        done += 1;
    }
    done
}

/// Reads one component's value out of the world as raw bytes. Used by tests and by the
/// demo to verify a migration produced what it claimed.
#[must_use]
pub fn read_raw(world: &World, entity: EntityView<'_>, component: &str) -> Option<Vec<u8>> {
    let id = lookup_component(world, component)?;
    let size = world
        .entity_from_id(id)
        .get::<&flecs_ecs::core::flecs::Component>(|c| usize::try_from(c.size).unwrap_or(0));
    if size == 0 {
        return None;
    }
    unsafe {
        let ptr = sys::ecs_get_id(world.ptr_mut(), *entity.id(), id);
        if ptr.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec())
    }
}
