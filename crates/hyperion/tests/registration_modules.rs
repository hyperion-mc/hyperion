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

use flecs_ecs::core::{ComponentId, World, WorldGet, id};
use hyperion::{
    egress::{
        ping::{Ping, PingComponentsModule},
        sync_chunks::{ChunkSendQueue, SyncChunksComponentsModule},
        tab_list::{TabList, TabListComponentsModule, Tps},
    },
    ingress::{
        IngressComponentsModule, PendingRemove, ServerPingResponse,
        decode::{DecodeComponentsModule, Decompressor},
    },
    simulation::{IgnMap, Player, SimComponentsModule, inventory::InventoryComponentsModule},
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

#[test]
#[serial]
fn tab_list_components_module_registers_both_singletons_standalone() {
    let world = World::new();
    world.import::<TabListComponentsModule>();

    assert_registered::<TabList>(&world);
    assert_registered::<Tps>(&world);
    // Both defaults are installed by the module that registers the type, so a
    // `get` reaching them at all is the assertion.
    world.get::<&TabList>(|_| ());
    world.get::<&Tps>(|_| ());

    assert!(
        world.try_lookup("tab_list_sample").is_none(),
        "a registration module must install no systems"
    );
    assert!(
        world.try_lookup("tab_list_sync").is_none(),
        "a registration module must install no systems"
    );
}

#[test]
#[serial]
fn ping_components_module_registers_the_readout_standalone() {
    let world = World::new();
    world.import::<PingComponentsModule>();

    assert_registered::<Ping>(&world);

    assert!(
        world.try_lookup("probe_ping").is_none(),
        "a registration module must install no systems"
    );
    assert!(
        world.try_lookup("publish_ping").is_none(),
        "a registration module must install no systems"
    );
}

/// The whole simulation registration layer, standing on its own.
///
/// The premise of every test above, applied to the module they all sit under.
/// It did not hold: `Prev` and `IgnMap` were registered only by
/// `HyperionCore`, so importing `SimComponentsModule` into a bare world -- the
/// thing this convention exists to make possible -- aborted a dev build with
/// `ECS_INVALID_OPERATION` before reaching the first assertion. Both are now
/// registered by the simulation's own DAG, and this is the guard that keeps
/// the next one from going unnoticed until a consumer trips over it.
#[test]
#[serial]
fn sim_components_module_stands_alone() {
    let world = World::new();
    world.import::<SimComponentsModule>();

    assert_registered::<Player>(&world);
    assert_registered::<IgnMap>(&world);

    // A `Player` carries a `Ping` without anyone on the join path adding one.
    // The trait is declared here rather than by `PingComponentsModule` because
    // it is a statement about `Player`; the assertion is here for the same
    // reason.
    let player = world.entity().add(id::<Player>());
    assert!(
        player.has(id::<Ping>()),
        "a Player should carry a Ping without anyone adding one"
    );

    assert!(
        world.try_lookup("probe_ping").is_none(),
        "a registration module must install no systems"
    );
}
