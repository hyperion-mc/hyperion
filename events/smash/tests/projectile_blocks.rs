//! Arrows stop at walls.
//!
//! Projectiles were point particles that flew through the arena until their
//! timer ran out, so every shot on every map was taken as if the geometry were
//! not there: a player behind a pillar was as shootable as one in the open.
//! These are the game-level half of the fix. The traversal itself is unit
//! tested in `crates/geometry/src/sweep.rs`; what is checked here is that a
//! whole `smash` world, driven through its own tick, puts the arrow in the wall
//! and leaves the player behind it alone.

mod harness;

use flecs_ecs::prelude::*;
use glam::{IVec3, Vec3};
use harness::{Game, TICK};
use smash::{
    module::{
        blocks::{BlockWorldHandle, Cubes},
        damage::{DamageKind, Damaged, hurt},
        player::{Health, Position},
        projectile::{Flight, Payload, Projectile, Stuck, Visual, fire},
    },
    server::{Sound, SoundCategory, mock::Call},
};

/// hyperion's `EntityKind::Arrow`, which is what a real bow fires.
const fn arrow_visual() -> Visual {
    Visual(hyperion::simulation::entity_kind::EntityKind::Arrow)
}

/// A world whose only terrain is a wall standing in the plane `x == 10`,
/// two blocks either side of the shooting line.
fn walled(game: &Game) {
    game.world.set(BlockWorldHandle::new(Cubes::wall(
        IVec3::new(10, -2, -2),
        IVec3::new(10, 4, 2),
    )));
}

/// The state of every projectile left in the world, as `(position, stuck)`.
fn projectiles(game: &Game) -> Vec<(Vec3, bool)> {
    let mut found = Vec::new();
    game.world
        .query::<&Flight>()
        .with(Projectile::id())
        .build()
        .each_entity(|entity, flight| {
            found.push((flight.position, entity.has(Stuck::id())));
        });
    found
}

/// Fire one flat, fast arrow along +X from the origin.
///
/// Sixty blocks a second is a full-draw Barrage arrow, which is three blocks a
/// tick: fast enough that a one-block wall sits between two consecutive
/// endpoints and only the cells between them say it is there.
fn shoot(game: &Game, shooter: Entity) {
    let shooter = game.world.entity_from_id(shooter);
    fire(
        shooter.world(),
        shooter,
        arrow_visual(),
        Flight {
            position: Vec3::new(0.5, 0.0, 0.5),
            velocity: Vec3::X * 60.0,
            gravity: 0.0,
            seconds_left: 3.0,
            radius: 0.4,
        },
        Payload::new(6.0, 1.0),
    );
}

#[test]
fn an_arrow_stops_at_the_wall_instead_of_passing_through_it() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));

    shoot(&game, shooter);
    // One tick moves it three blocks, so it takes four to reach x == 10 --
    // and every one of those ticks is a three-block step that a point sample
    // would have skipped two thirds of.
    game.advance(TICK * 6.0, 6);

    let left = projectiles(&game);
    assert_eq!(
        left.len(),
        1,
        "the arrow should still exist, stuck: {left:?}"
    );
    let (at, stuck) = left[0];
    assert!(stuck, "an arrow that met a wall is stuck in it: {left:?}");
    assert!(
        (at.x - 10.0).abs() < 1e-3,
        "the arrow stopped at x = {}, and the wall's near face is x = 10",
        at.x
    );
}

#[test]
fn an_arrow_over_open_ground_flies_exactly_as_it_did_before() {
    let mut game = Game::new();
    // No terrain seam installed: `OpenAir`, which is what every other test in
    // this directory runs with.
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));

    shoot(&game, shooter);
    game.advance(TICK * 6.0, 6);

    let left = projectiles(&game);
    assert_eq!(left.len(), 1, "nothing stops it: {left:?}");
    let (at, stuck) = left[0];
    assert!(!stuck, "there is nothing to stick in: {left:?}");
    // Six ticks at sixty blocks a second is eighteen blocks, well past where
    // the wall stood in the test above.
    assert!(
        at.x > 17.0,
        "the arrow should be eighteen blocks out, it is at x = {}",
        at.x
    );
}

#[test]
fn a_player_behind_the_wall_is_not_hit_through_it() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));
    // Standing one block past the wall, dead on the shooting line. Before the
    // sweep the arrow crossed the wall in a single step and took them with it.
    let victim = game.player("victim", Vec3::new(11.5, 0.0, 0.5));

    shoot(&game, shooter);
    game.advance(TICK * 6.0, 6);

    let health = game
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| health.current)
        .expect("a player has health");
    let max = game
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| health.max)
        .expect("a player has health");
    assert!(
        (health - max).abs() < 1e-6,
        "the victim was shot through a wall: {health} of {max}"
    );

    // The control, in the same test, because without it this passes for a
    // world where the arrow could never have reached them anyway -- a victim
    // half a block off the flight line, a wall in the wrong place, a `fire`
    // that silently did nothing. The identical setup with no wall must hit.
    let mut open = Game::new();
    let shooter = open.player("shooter", Vec3::new(0.0, 0.0, 0.0));
    let victim = open.player("victim", Vec3::new(11.5, 0.0, 0.5));
    shoot(&open, shooter);
    open.advance(TICK * 6.0, 6);

    let (health, max) = open
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| (health.current, health.max))
        .expect("a player has health");
    assert!(
        health < max,
        "the control shot missed too, so the test above proves nothing: {health} of {max}"
    );
}

#[test]
fn a_player_in_front_of_the_wall_is_still_hit() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));
    // The mirror image of the test above: same wall, victim on this side of
    // it. Clipping the segment must not clip away the hits that should land.
    let victim = game.player("victim", Vec3::new(6.0, 0.0, 0.5));

    shoot(&game, shooter);
    game.advance(TICK * 6.0, 6);

    let health = game
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| health.current)
        .expect("a player has health");
    let max = game
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| health.max)
        .expect("a player has health");
    assert!(
        health < max,
        "the victim stood in the open and took nothing: {health} of {max}"
    );
}

#[test]
fn the_impact_is_audible_at_the_point_it_happened() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));

    shoot(&game, shooter);
    game.server.take();
    game.advance(TICK * 6.0, 6);

    let impacts: Vec<Vec3> = game
        .server
        .calls()
        .iter()
        .filter_map(|call| match call {
            Call::Sound(at, sound)
                if *sound
                    == Sound::new(smash::module::sound::PROJECTILE_HIT, SoundCategory::Neutral) =>
            {
                Some(*at)
            }
            _ => None,
        })
        .collect();

    assert_eq!(impacts.len(), 1, "one arrow, one impact: {impacts:?}");
    assert!(
        (impacts[0].x - 10.0).abs() < 1e-3,
        "the sound played at x = {}, and the wall's near face is x = 10",
        impacts[0].x
    );
}

#[test]
fn a_stuck_arrow_does_not_shoot_whoever_walks_into_it() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));
    let bystander = game.player("bystander", Vec3::new(30.0, 0.0, 30.0));

    shoot(&game, shooter);
    game.advance(TICK * 6.0, 6);
    assert!(
        projectiles(&game).iter().any(|(_, stuck)| *stuck),
        "the arrow should be stuck before this test means anything"
    );

    // Walk them onto it. A stuck projectile is excluded from the flight system
    // outright, so there is nothing left to hit them with.
    game.world
        .entity_from_id(bystander)
        .set(Position(Vec3::new(9.8, 0.0, 0.5)));
    game.advance(TICK * 4.0, 4);

    let health = game
        .world
        .entity_from_id(bystander)
        .try_get::<&Health>(|health| health.current)
        .expect("a player has health");
    let max = game
        .world
        .entity_from_id(bystander)
        .try_get::<&Health>(|health| health.max)
        .expect("a player has health");
    assert!(
        (health - max).abs() < 1e-6,
        "a stuck arrow shot a passer-by: {health} of {max}"
    );
}

#[test]
fn a_stuck_arrow_stops_existing() {
    let mut game = Game::new();
    walled(&game);
    let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 0.0));

    shoot(&game, shooter);
    game.advance(TICK * 6.0, 6);
    assert!(
        !projectiles(&game).is_empty(),
        "it should be stuck in the wall at this point"
    );

    // Past `STUCK_SECONDS`. Left forever, a Barrage volley would carpet the
    // far wall for the rest of the match.
    game.advance(smash::module::projectile::STUCK_SECONDS + TICK, 25);
    assert!(
        projectiles(&game).is_empty(),
        "the arrow is still in the wall: {:?}",
        projectiles(&game)
    );
}

/// The damage path is untouched by any of this: a hit is still a hit.
#[test]
fn direct_damage_is_unaffected() {
    let mut game = Game::new();
    walled(&game);
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(1.0, 0.0, 0.0));

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: Some(attacker),
        amount: 5.0,
        knockback: smash::module::knockback::Knockback::from(Vec3::ZERO),
        kind: DamageKind::Projectile,
    });
    game.advance(TICK, 1);

    let health = game
        .world
        .entity_from_id(victim)
        .try_get::<&Health>(|health| health.current)
        .expect("a player has health");
    assert!(health < 20.0, "the victim took nothing: {health}");
}
