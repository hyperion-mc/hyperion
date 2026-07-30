//! Answers one question: do a host binary and a module dylib draw component indices from
//! one shared pool?
//!
//! `flecs_ecs`'s derive emits, per component type, a `static INDEX` initialised from a
//! process-global `INDEX_POOL`, and that index is a slot in the world's component array.
//! Two copies of `flecs_ecs` in one process means two pools, so the module writes into a
//! slot the host never filled. Everything the hot-reload gate does rests on this being one
//! pool, and nothing else it checks would notice if it were not.
//!
//! The test is behavioural rather than an address comparison, deliberately. Comparing
//! `ecs_init as usize` across the boundary reports a difference even when the copy is
//! shared, because an executable taking the address of a dynamically-linked function gets
//! its own PLT stub rather than the implementation. Measured exactly that trap: separate
//! copies and shared copies both printed mismatched addresses. Allocation order cannot be
//! faked -- if the pool is shared, an index taken here is strictly greater than every
//! index the host took first.
//!
//! The equality of any single index is not evidence either, and looked like evidence once.
//! Two separate pools both start at 1, so a type that happens to be the first registered on
//! each side reads `1` and `1`. That is what this probe reported when the module linked its
//! own static copy of everything, which is exactly the case it exists to detect.
//!
//! What makes the pool shared is `flecs_ecs` being built as a dylib, so every consumer
//! resolves one `libflecs_ecs.so`. It is not this crate's dependency list: dropping the
//! `hyperion-hot-reload` dependency entirely leaves the probe passing.

use flecs_ecs::core::ComponentId;

/// Declared here so its index can only ever have been allocated by this dylib.
#[derive(flecs_ecs::macros::Component)]
pub struct ModuleOnlyMarker;

/// A component type `hyperion` owns, to check the shared-pool result holds for a type
/// neither side declares locally.
///
/// # Safety
/// Called by the probe host through `dlsym`. Returns a plain integer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probe_position_index() -> u32 {
    <hyperion::simulation::Position as ComponentId>::index()
}

/// An index allocated in this dylib, after the host has already taken several.
///
/// # Safety
/// Called by the probe host through `dlsym`. Returns a plain integer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn probe_module_index() -> u32 {
    <ModuleOnlyMarker as ComponentId>::index()
}
