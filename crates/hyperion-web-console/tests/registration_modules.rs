//! Dev-profile guard: the console's registration module stands on its own.
//!
//! The same property `crates/hyperion/tests/registration_modules.rs` asserts
//! for the engine's modules, asserted for this crate's. It has to live here
//! rather than beside those: `hyperion` cannot depend on a crate that depends
//! on `hyperion`.
//!
//! This is the only shape of test that can see ENG-11000's class. flecs builds
//! with `flecs_manual_registration`, so using a component before registering it
//! aborts with `ECS_INVALID_OPERATION` -- but that guard is an `ecs_assert`,
//! which release builds compile out. Every e2e gate in this repo builds a
//! release binary and is therefore structurally blind to it. `cargo test` is
//! not.

use flecs_ecs::core::{ComponentId, World, id};
use hyperion_web_console::{
    ConsoleComponentsModule,
    module::{ConsoleCaller, ConsoleHandle},
};
use serial_test::serial;

/// Asserts `T` is registered without registering it as a side effect, which is
/// what a plain `id::<T>()` or a `set` would do.
fn assert_registered<T: ComponentId>(world: &World) {
    assert!(
        world.get_component_id::<T>().is_some(),
        "{} should be registered by its registration module alone",
        core::any::type_name::<T>()
    );
}

#[test]
#[serial]
fn the_registration_module_registers_both_singletons_standalone() {
    let world = World::new();
    world.import::<ConsoleComponentsModule>();

    assert_registered::<ConsoleHandle>(&world);
    assert_registered::<ConsoleCaller>(&world);
}

#[test]
#[serial]
fn registration_carries_no_behaviour() {
    // The other half of the convention in the root `CLAUDE.md`: a consumer can
    // import the types without importing the systems. If these ever appear
    // here, "give me the components but not the behaviour" has stopped being
    // expressible and the smash mock loses the seam it depends on.
    let world = World::new();
    world.import::<ConsoleComponentsModule>();

    assert!(
        world.try_lookup("console_requests").is_none(),
        "the registration module must install no systems"
    );
    assert!(
        world.try_lookup("console_snapshot").is_none(),
        "the registration module must install no systems"
    );
}

/// The singletons are registered, but deliberately not *set*.
///
/// Unlike `SpatialIndex` or `WorldTime`, neither of these has a meaningful
/// default: a `ConsoleHandle` is an `Arc<Console>` that only exists once an
/// operator has asked for a console and a port has been bound, and a
/// `ConsoleCaller` names an entity that `install` creates. So the registration
/// module registers the types and `install` sets the values, and that split is
/// the thing this asserts -- a future edit that "helpfully" adds a `Default`
/// and sets it here would give every server a console handle pointing at
/// nothing.
#[test]
#[serial]
fn the_singletons_are_registered_but_not_set() {
    let world = World::new();
    world.import::<ConsoleComponentsModule>();

    assert!(
        !world.has(id::<ConsoleHandle>()),
        "a handle must not exist until `install` has bound a port"
    );
    assert!(
        !world.has(id::<ConsoleCaller>()),
        "a caller must not exist until `install` has spawned the entity"
    );
}
