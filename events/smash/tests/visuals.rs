//! Every ability draws something, and no two things that hurt you look alike.
//!
//! An invisible ability is the same bug as a silent one: the damage lands, the
//! knockback packet goes out, and the only symptom is that the player cannot
//! see what hit them. `Cue` made that bug structural rather than accidental --
//! five variants stood in for every visual in the game, so a kit could not draw
//! anything Mineplex had not already needed. These are the assertions that keep
//! the replacement honest now that a kit can draw all 125.
//!
//! Checked on the `MockServer` call log, which is the same seam
//! `tests/sound.rs` reads, and for the same reason: a scripted client cannot
//! assert that a human found the effect legible, but it can assert that the
//! packet carrying it was asked for at all.

mod harness;

use glam::Vec3;
use harness::Game;
use smash::{
    module::{damage::DamageKind, effect, visuals},
    server::{Particle, Particles},
};

/// The particle every point of an effect draws.
fn drawn(effect: &Particles) -> Particle<'_> {
    effect
        .packets()
        .next()
        .expect("an effect with no points draws nothing")
        .particle
}

/// Burn and poison are told apart by their picture and by nothing else.
///
/// Both take a point of health a second off somebody standing still. When both
/// were `Cue`s the game had no way to draw them differently, and a player could
/// not tell a Blaze from a Spider. This is the assertion that says the two are
/// still distinct after any future tidy-up of `visuals`.
#[test]
fn a_burn_and_a_poison_do_not_look_the_same() {
    let at = Vec3::new(0.0, 64.0, 0.0);
    assert_ne!(drawn(&visuals::burn(at)), drawn(&visuals::venom(at)));
}

/// The two that were placeholders are now the particles vanilla actually uses.
///
/// `burn` was pinned to `crit` and `venom` to half-power `dragon_breath` for as
/// long as the protocol layer could spell five particles. Naming the real ones
/// here is what stops a future edit quietly reverting to the nearest thing.
#[test]
fn a_burn_is_flame_and_a_poison_is_an_effect_tint() {
    let at = Vec3::new(0.0, 64.0, 0.0);
    assert_eq!(drawn(&visuals::burn(at)), Particle::Flame);
    assert!(
        matches!(drawn(&visuals::venom(at)), Particle::EntityEffect { .. }),
        "a poison is a tinted potion effect, not a shape"
    );
}

/// Every visual puts its particles where it was asked to, near enough that a
/// player reads it as happening to them.
///
/// An effect drawn at the feet of somebody two blocks away marks the wrong
/// player. The bound is a player's own height: 1.8 blocks tall, and these draw
/// from the feet up to a shell over the head, so everything should land inside
/// roughly one player-sized box around the point it was given.
#[test]
fn every_visual_is_drawn_where_it_was_asked_for() {
    let at = Vec3::new(12.5, 64.0, -3.5);
    for (name, effect) in [
        ("blast", visuals::blast(at)),
        ("teleport", visuals::teleport(at)),
        ("death", visuals::death(at)),
        ("burn", visuals::burn(at)),
        ("venom", visuals::venom(at)),
    ] {
        let packets: Vec<_> = effect.packets().collect();
        assert!(!packets.is_empty(), "{name} draws nothing");
        for packet in &packets {
            // The packet carries doubles; the game's own positions are f32,
            // so narrowing back is exactly the round trip a caller made.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the position went in as an f32 and the packet only widened it"
            )]
            let drawn_at = Vec3::new(packet.x as f32, packet.y as f32, packet.z as f32);
            assert!(
                drawn_at.distance(at) <= 2.5,
                "{name} drew a particle {:.2} blocks from {at:?}, at {drawn_at:?}",
                drawn_at.distance(at)
            );
            assert!(packet.count > 0, "{name} sends a packet that draws nothing");
        }
    }
}

/// An affliction that ticks draws the picture its `Shows` names.
///
/// This is the assertion that was missing when `Cue` came out. Deleting the
/// enum moved the picture from a field the compiler checked to a function
/// pointer it also checks, but nothing anywhere said the picture still
/// *arrived* -- an affliction whose particle call was dropped would tick, hurt,
/// make its noise, and be invisible, and every test in the suite would pass.
#[test]
fn an_affliction_draws_the_picture_it_names() {
    let mut game = Game::new();
    let player = game.player("burned", Vec3::new(0.0, 64.0, 0.0));

    effect::afflict(
        (&game.world).into(),
        game.world.entity_from_id(player),
        effect::Blame {
            source: player,
            attacker: player,
        },
        effect::Affliction::over_time(1.0, 2.0, 0.1, DamageKind::Environment, effect::Shows {
            effect: visuals::burn,
            sound: "minecraft:entity.player.hurt_on_fire",
        }),
    );
    game.server.take();
    game.advance(0.4, 4);

    let drawn_effects = game.server.particles();
    assert!(
        !drawn_effects.is_empty(),
        "a burn ticked and nothing was drawn"
    );
    assert_eq!(
        drawn(&drawn_effects[0]),
        Particle::Flame,
        "the affliction drew something other than the picture it names"
    );
}
