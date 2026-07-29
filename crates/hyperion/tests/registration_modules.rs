//! Dev-profile guard: every registration module stands on its own.
//!
//! The flecs convention (root `CLAUDE.md`) says a registration module can be
//! imported into a bare world and leave every component it owns registered,
//! with no systems and no engine boot. That is what makes "give me the types,
//! not the behavior" expressible, and it is the property ENG-11000's class
//! breaks: a component used before it is registered aborts a dev build with
//! `ECS_INVALID_OPERATION` and compiles clean in release, so only a
//! debug-assertions test like this one sees it.
//!
//! Each test imports exactly one registration module into an otherwise empty
//! world and asks flecs which components exist. `get_component_id` answers
//! without registering anything itself, so a dropped `world.component::<T>()`
//! shows up as a failed assertion naming the type rather than as an abort with
//! no attribution.

use flecs_ecs::core::{ComponentId, World, WorldGet};
use hyperion::{
    egress::sync_chunks::{ChunkSendQueue, SyncChunksComponentsModule},
    ingress::{
        IngressComponentsModule, PendingRemove, ServerPingResponse,
        decode::{DecodeComponentsModule, Decompressor},
    },
    simulation::inventory::InventoryComponentsModule,
    spatial::{Spatial, SpatialComponentsModule, SpatialIndex},
};
use hyperion_inventory::{CursorItem, InventoryState, OpenInventory, PlayerInventory};
use serial_test::serial;

/// Asserts `T` is registered in `world` without registering it as a side
/// effect, which a plain `id::<T>()` or `set` would do.
fn assert_registered<T: ComponentId>(world: &World) {
    assert!(
        world.get_component_id::<T>().is_some(),
        "{} should be registered by its registration module alone",
        core::any::type_name::<T>()
    );
}

#[test]
#[serial]
fn spatial_components_module_registers_the_index_standalone() {
    let world = World::new();
    world.import::<SpatialComponentsModule>();

    assert_registered::<Spatial>(&world);
    assert_registered::<SpatialIndex>(&world);
    // The singleton default has to be installed by the same module that
    // registers the type. A `get` of an unset or unregistered singleton is the
    // dev-build abort, so reaching it at all is the assertion.
    world.get::<&SpatialIndex>(|_| ());

    // The other half of the convention: registration carries no behavior, so
    // the rebuild system that `SpatialModule` installs must not be here.
    assert!(
        world.try_lookup("recalculate_spatial_index").is_none(),
        "a registration module must install no systems"
    );
}

#[test]
#[serial]
fn decode_components_module_registers_the_frame_path_standalone() {
    let world = World::new();
    world.import::<DecodeComponentsModule>();

    assert_registered::<packet_channel::Receiver>(&world);
    assert_registered::<Decompressor>(&world);
    world.get::<&Decompressor>(|_| ());
}

#[test]
#[serial]
fn sync_chunks_components_module_registers_the_queue_standalone() {
    let world = World::new();
    world.import::<SyncChunksComponentsModule>();

    assert_registered::<ChunkSendQueue>(&world);
    world.entity().set(ChunkSendQueue::default());
}

#[test]
#[serial]
fn inventory_components_module_registers_every_inventory_type_standalone() {
    let world = World::new();
    world.import::<InventoryComponentsModule>();

    // All four, because `SimComponentsModule` declares `Player`-implies-
    // `CursorItem` and `Player`-implies-`InventoryState` traits that point at
    // two of them: a relation target has to be a registered entity first.
    assert_registered::<PlayerInventory>(&world);
    assert_registered::<CursorItem>(&world);
    assert_registered::<InventoryState>(&world);
    assert_registered::<OpenInventory>(&world);

    world.entity().set(PlayerInventory::default());
}

#[test]
#[serial]
fn ingress_components_module_registers_the_inbound_edge_standalone() {
    let world = World::new();
    world.import::<IngressComponentsModule>();

    assert_registered::<PendingRemove>(&world);
    assert_registered::<ServerPingResponse>(&world);
    world.get::<&ServerPingResponse>(|_| ());
}
