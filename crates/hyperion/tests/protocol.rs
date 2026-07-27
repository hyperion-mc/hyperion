//! What a 26.2 client needs from the configuration state before it can join.
//!
//! These are the checks whose failure mode is a client that hangs or is kicked
//! with a message pointing somewhere else entirely, so they are worth asserting
//! rather than discovering from a transcript.

use hyperion::net::protocol::{Clientbound, registries};
use hyperion_minecraft_proto::{
    Decode, Reader,
    packets::play_login::{CommonPlayerSpawnInfo, GameType, Login},
};

/// `RegistryDataLoader.SYNCHRONIZED_REGISTRIES` marks fifteen registries with
/// `RegistryValidator.nonEmpty()`, and the client runs that validator over what
/// the server sent. An empty one is a kick during the handover, not a missing
/// texture later.
#[test]
fn every_synchronized_registry_has_entries() {
    for registry in registries::SYNCHRONIZED {
        assert!(
            !registry.entries.is_empty(),
            "{} was sent with no elements",
            registry.name
        );
    }
}

/// Every element must be namespaced. An unnamespaced id is a parse error on the
/// client, which reports it as a corrupt registry rather than as a bad name.
#[test]
fn every_registry_element_is_namespaced() {
    for registry in registries::SYNCHRONIZED {
        for entry in registry.entries {
            assert!(
                entry.contains(':'),
                "{} carries an unnamespaced element {entry}",
                registry.name
            );
        }
    }
}

/// The join packet indexes `minecraft:dimension_type` by network id, so the
/// level this server serves has to be in the table it sends.
#[test]
fn the_served_dimension_has_a_network_id() {
    assert_eq!(
        registries::DIMENSION_TYPE.id_of("minecraft:overworld"),
        Some(0),
        "dimension_type must contain the overworld"
    );
}

/// A registry sent twice would give its elements two network ids, and the
/// client keeps whichever arrived last.
#[test]
fn no_registry_is_sent_twice() {
    let mut names: Vec<_> = registries::SYNCHRONIZED
        .iter()
        .map(|registry| registry.name)
        .collect();
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(before, names.len(), "a registry appears more than once");
}

/// The wire form is the packet id and then the body, with nothing between.
///
/// This is the whole of the adapter between the proto crate's `Encode` and
/// hyperion's framing, and getting it wrong shifts every field of every packet
/// by one byte.
#[test]
fn clientbound_writes_the_id_then_the_body() {
    use hyperion::PacketBundle;

    let login = Login {
        player_id: 7,
        hardcore: false,
        levels: vec!["minecraft:overworld"],
        max_players: 12_000,
        chunk_radius: 10,
        simulation_distance: 10,
        reduced_debug_info: false,
        show_death_screen: false,
        do_limited_crafting: false,
        spawn_info: CommonPlayerSpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld",
            seed: 0,
            game_type: GameType::Survival,
            previous_game_type: None,
            is_debug: false,
            is_flat: false,
            last_death_location: None,
            portal_cooldown: 0,
            sea_level: 63,
        },
        online_mode: false,
        enforces_secure_chat: false,
    };

    let mut encoded = Vec::new();
    Clientbound::new(0x31, &login)
        .encode_including_ids(&mut encoded)
        .expect("a packet this server built must encode");

    let mut reader = Reader::new(&encoded);
    assert_eq!(reader.var_int().expect("packet id"), 0x31);

    let decoded = Login::decode(&mut reader).expect("body must decode");
    reader
        .finish()
        .expect("body must consume the rest of the frame");
    assert_eq!(decoded.player_id, 7);
    assert_eq!(decoded.spawn_info.dimension, "minecraft:overworld");
}
