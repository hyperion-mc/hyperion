//! A dev-profile boot regression for the frozen-daylight singleton.
//!
//! `WorldTime` is set on join by the join path's `world.get::<&WorldTime>`.
//! When it was installed with a bare `world.set` and no
//! `add_trait::<flecs::Singleton>()`, a release build read it fine but a dev
//! build asserted "component is not registered" the first time a player
//! joined -- so the smash server crash-looped on boot under `nix run .#smash`
//! (the dev profile) while every release gate stayed green. This test is that
//! join-path read in miniature: it boots `HyperionCore` and reads the
//! singleton, which panics on the unregistered component in exactly the dev
//! build `cargo test` produces. It fails without the registration and passes
//! with it.

use flecs_ecs::core::{World, WorldGet};
use hyperion::simulation::{WorldTime, world_time};
use serial_test::serial;

#[test]
#[serial]
fn world_time_singleton_is_registered_after_core_boot() {
    let world = World::new();
    world.import::<hyperion::HyperionCore>();

    // The join path does exactly this. A `world.get` of a singleton that was
    // never registered with the `Singleton` trait aborts a dev build, which is
    // the crash the smash operator hit.
    let day_time = world.get::<&WorldTime>(|world_time| world_time.day_time);

    assert_eq!(
        day_time,
        world_time::NOON,
        "HyperionCore should install the WorldTime singleton at its noon default"
    );
}
