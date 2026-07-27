//! `PacketState` used to be four marker components (`packet_state::Handshake` and friends). It is
//! now a flecs enum, so protocol state is an exclusive relationship pair rather than a tag, and
//! every decode system selects on it with `with_enum`. These tests pin that behaviour: selecting
//! one state must not match another, and moving to the next state must retract the previous one.

use flecs_ecs::core::{Builder, QueryAPI, QueryBuilderImpl, World};
use hyperion::simulation::PacketState;

#[test]
fn with_enum_selects_only_the_current_state() {
    let world = World::new();
    world.component::<PacketState>();

    world.entity().add_enum(PacketState::Handshake);
    world.entity().add_enum(PacketState::Play);

    let handshake = world
        .query::<()>()
        .with_enum(PacketState::Handshake)
        .build();
    let play = world.query::<()>().with_enum(PacketState::Play).build();
    let login = world.query::<()>().with_enum(PacketState::Login).build();

    assert_eq!(handshake.count(), 1);
    assert_eq!(play.count(), 1);
    assert_eq!(login.count(), 0);
}

#[test]
fn advancing_state_retracts_the_previous_one() {
    let world = World::new();
    world.component::<PacketState>();

    let player = world.entity().add_enum(PacketState::Handshake);

    // ingress::process_handshake relies on this: it adds the next state without removing the
    // handshake state first.
    player.add_enum(PacketState::Login);

    let handshake = world
        .query::<()>()
        .with_enum(PacketState::Handshake)
        .build();
    let login = world.query::<()>().with_enum(PacketState::Login).build();

    assert_eq!(
        handshake.count(),
        0,
        "PacketState must be exclusive, or a player would be decoded as two states at once"
    );
    assert_eq!(login.count(), 1);
}
