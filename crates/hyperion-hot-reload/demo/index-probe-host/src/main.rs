//! Compares component-index allocation across the host/module dylib boundary.
//! See the module crate for why this is behavioural rather than an address comparison.
#![allow(
    clippy::print_stdout,
    reason = "this binary's whole purpose is reporting what it measured"
)]

use flecs_ecs::core::ComponentId;

#[derive(flecs_ecs::macros::Component)]
struct HostMarkerA;
#[derive(flecs_ecs::macros::Component)]
struct HostMarkerB;
#[derive(flecs_ecs::macros::Component)]
struct HostMarkerC;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: hot-reload-index-probe <module dylib>");

    // Take several indices before the module is even loaded, so a shared pool has visibly
    // advanced by the time the module asks for one.
    let host_indices = [
        <HostMarkerA as ComponentId>::index(),
        <HostMarkerB as ComponentId>::index(),
        <HostMarkerC as ComponentId>::index(),
        <hyperion::simulation::Position as ComponentId>::index(),
    ];
    let host_max = host_indices.iter().copied().max().expect("non-empty");

    let lib = unsafe { libloading::Library::new(&path) }.expect("failed to dlopen the module");
    let module_index = unsafe {
        let f: libloading::Symbol<'_, unsafe extern "C" fn() -> u32> = lib
            .get(b"probe_module_index")
            .expect("no module index symbol");
        f()
    };
    let module_position_index = unsafe {
        let f: libloading::Symbol<'_, unsafe extern "C" fn() -> u32> = lib
            .get(b"probe_position_index")
            .expect("no position index symbol");
        f()
    };
    // Leaked deliberately: `dlclose` would unmap text the process still holds pointers
    // into, which is the segfault-at-exit documented in docs/hot-reload.md.
    core::mem::forget(lib);

    println!("host indices: {host_indices:?} (max {host_max})");
    println!("module's own type index: {module_index}");
    println!(
        "hyperion::simulation::Position index: host {}, module {module_position_index}",
        host_indices[3]
    );

    let shared_pool = module_index > host_max;
    let shared_hyperion_index = host_indices[3] == module_position_index;
    println!("SHARED_POOL={shared_pool}");
    println!("SHARED_HYPERION_INDEX={shared_hyperion_index}");

    assert!(
        shared_pool,
        "host and module allocate component indices from separate pools: the module got \
         {module_index} after the host had already taken up to {host_max}"
    );
    println!("PROBE_OK");
}
