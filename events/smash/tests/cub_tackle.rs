//! Cub Tackle leaves a real slow, not just a marker.
//!
//! The tooltip promises "whoever it lands on can barely move for five seconds".
//! Wolf Strike already read a [`Tackled`] marker for its combo, but nothing was
//! ever sent to slow the victim -- the tooltip lied. This pins both halves: the
//! marker the combo needs, and the `Slowness VI` the client applies to its own
//! movement prediction so the victim actually can barely move.
//!
//! Two tests, because they fail differently. One drives the whole ability
//! through the mock seam and asserts the victim was slowed; the other pins the
//! exact bytes the effect encodes to, the way `hyperion`'s own
//! `play_mob_effect` differential pins them against Mojang's encoder.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use hyperion_minecraft_proto::{Encode, Writer};
use smash::{
    module::{
        ability,
        damage::MatchClock,
        kit,
        kits::wolf::{self, Tackled},
    },
    server::PlayerId,
};

fn encoded(value: &impl Encode) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

/// Fired at a victim standing in front, the cub lands and the victim is slowed
/// with exactly the effect [`wolf::tackle_slow`] describes -- and the combo
/// marker is still there beside it, not replaced by it.
#[test]
fn a_cub_slows_whoever_it_lands_on_and_still_marks_them() {
    let mut game = Game::new();
    let caster = game.player("wolf", Vec3::ZERO);
    // Straight down the caster's +X facing, close enough that the cub lands
    // almost at once and well inside the tick budget below.
    let victim = game.player("victim", Vec3::new(3.5, 0.0, 0.0));

    let wolf_kit = kit::by_name(&game.world, "Wolf").expect("the registry has Wolf");
    kit::apply(&game.world, game.world.entity_from_id(caster), wolf_kit);
    // A non-zero match clock so the marker's deadline is a real future time.
    game.world.set(MatchClock(1.0));

    let entry = ability::manifest(&game.world)
        .into_iter()
        .find(|entry| entry.kit == "Wolf" && entry.name == "Cub Tackle")
        .expect("Wolf declares Cub Tackle");
    ability::use_slot(game.world.entity_from_id(caster), entry.slot);

    // Long enough for the cub to cross 3.5 blocks at 18 blocks a second.
    game.advance(1.5, 30);

    let victim_id: PlayerId = game.world.entity_from_id(victim).cloned::<&PlayerId>();
    let slowed = game.server.statuses_of(victim_id);
    assert_eq!(
        slowed,
        vec![wolf::tackle_slow()],
        "the cub landed and the victim was not slowed as the tooltip promises: {slowed:?}"
    );
    assert!(
        game.world.entity_from_id(victim).has(Tackled::id()),
        "the real slow replaced the combo marker instead of joining it"
    );

    // The caster threw the cub; it did not slow themselves.
    let caster_id: PlayerId = game.world.entity_from_id(caster).cloned::<&PlayerId>();
    assert!(
        game.server.statuses_of(caster_id).is_empty(),
        "the thrower slowed themselves"
    );
}

/// The effect encodes byte-for-byte to the layout `hyperion`'s `play_mob_effect`
/// differential pins against Mojang's own encoder: entity id, effect id
/// (`Slowness` is 1), amplifier (`5` is level VI, where a player can no longer
/// move), duration in ticks (five seconds is 100), and the flag byte
/// (visible | show-icon = `0b110`).
#[test]
fn the_slow_is_slowness_six_for_five_seconds_on_the_wire() {
    let packet = wolf::tackle_slow().packet(0x2A);
    assert_eq!(encoded(&packet), [0x2A, 0x01, 0x05, 0x64, 0x06]);
}
