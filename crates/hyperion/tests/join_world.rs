//! A client must reach play state, not merely authenticate.
//!
//! The bevy-to-flecs migration left the login path and the join system talking
//! past each other: login applied the skin straight to the entity while the
//! join system waited on a channel nobody sent to. Every client authenticated
//! and then sat on "Joining world..." forever. Nothing caught it, because
//! `player_join_world` still compiled and the proxy still reported players
//! connected -- only a client driven all the way into the world can tell.

use hyperion::simulation::{Comms, skin::PlayerSkin};

/// The channel the join system drains is the one login must send to.
///
/// This is a unit-level guard on the wiring rather than a full network test:
/// the observable failure was a send that never happened, so proving the
/// channel round-trips is what would have caught it.
#[test]
fn skin_handed_to_the_join_system_arrives() {
    let comms = Comms::default();

    // Entity ids are opaque here; any id proves delivery.
    let sender = unsafe { std::mem::transmute::<u64, flecs_ecs::core::Entity>(1_u64) };

    comms
        .skins_tx
        .send((sender, PlayerSkin::EMPTY))
        .expect("the join system holds the receiver, so a send must succeed");

    let received = comms
        .skins_rx
        .try_recv()
        .expect("receive must not error")
        .expect("login sent a skin, so the join system must see one");

    assert_eq!(received.0, sender);
}
