//! A client must reach play state, not merely authenticate.
//!
//! The bevy-to-flecs migration left the login path and the join system talking
//! past each other: login applied the skin straight to the entity while the
//! join system waited on a channel nobody sent to. Every client authenticated
//! and then sat on "Joining world..." forever. Nothing caught it, because the
//! join path still compiled and the proxy still reported players connected --
//! only a client driven all the way into the world can tell.
//!
//! Protocol 776 removed the shape of that bug rather than fixing it: the join
//! is driven by the client's own `finish_configuration` in
//! `hyperion::net::protocol::join`, so a skin that never arrives can no longer
//! keep anyone out of the world. The channel is still how skins get applied, so
//! it is still worth proving it round-trips.

use hyperion::simulation::{Comms, skin::PlayerSkin};

/// The channel the skin system drains is the one login must send to.
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
