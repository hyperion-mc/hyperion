//! The map's edges: the hub shoves you back, the arena drops you.
//!
//! One system, two policies. These drive it against the mock -- no host -- so
//! what they read is the seam call the adapter would have turned into a packet:
//! an `add_velocity` pointing back inside for the hub, and a life lost for the
//! arena. `nix run .#smash-e2e` proves the same on the wire, where a test that
//! only checked the player ended up in bounds would also pass for the teleport
//! the operator ruled out; here and there, the assertion is on the push.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        arena::{Arena, Bounds, HubBounds, Policy},
        lives::Lives,
        lobby::{Lobby, Phase},
        player::Position,
    },
    server::{PlayerId, mock::Call},
};

/// A finite hub, so the push has a wall to act at. The harness defaults this to
/// an infinite box -- see its comment -- so a test that wants the wall says so.
fn walled_hub(game: &Game) {
    game.world.set(HubBounds(Bounds {
        min: Vec3::new(-21.0, 64.0, -21.0),
        max: Vec3::new(21.0, 80.0, 21.0),
        policy: Policy::PushBack,
    }));
}

/// Put the lobby in `phase` without waiting the state machine into it.
fn phase(game: &Game, phase: Phase) {
    game.world.set(Lobby { phase, timer: 0.0 });
}

/// The velocity the server was told to add to `player` this run, summed.
fn pushed(game: &Game, player: Entity) -> Vec3 {
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
    game.server.total_velocity(id)
}

/// A player who jumps out of the hub is shoved back through the nearest wall.
#[test]
fn the_hub_shoves_a_player_back_over_the_wall() {
    let mut game = Game::new();
    walled_hub(&game);
    phase(&game, Phase::Waiting);

    // Over the east wall (x past the box) and above it, as a double jump that
    // cleared the glass would leave them.
    let escapee = game.player("escapee", Vec3::new(25.0, 70.0, 0.0));
    game.world
        .entity_from_id(escapee)
        .set(Position(Vec3::new(25.0, 70.0, 0.0)));

    game.advance(0.05, 1);

    let push = pushed(&game, escapee);
    assert!(
        push.x < -1e-3,
        "a player east of the box should be shoved west, back inside; got {push:?}"
    );
    // Nearest-face, not toward-centre: they left over one wall and only that
    // wall pushes, so the shove is along -x with no z of its own.
    assert!(
        push.z.abs() < 1e-3,
        "the shove is the nearest-wall normal, so a player who left over the east wall gets no z \
         component; got {push:?}"
    );
}

/// The push is seen and heard, so it reads as a wall and not a glitch.
#[test]
fn the_shove_is_visible_and_audible() {
    let mut game = Game::new();
    walled_hub(&game);
    phase(&game, Phase::Waiting);

    let escapee = game.player("escapee", Vec3::new(0.0, 95.0, 0.0));
    game.world
        .entity_from_id(escapee)
        .set(Position(Vec3::new(0.0, 95.0, 0.0)));

    game.advance(0.05, 1);

    let calls = game.server.calls();
    assert!(
        calls.iter().any(|call| matches!(call, Call::Particles(_))),
        "the shove drew no particles, so it reads as the game rubber-banding rather than a wall"
    );
    assert!(
        calls.iter().any(|call| matches!(call, Call::Sound(..))),
        "the shove made no sound"
    );
}

/// A player standing safely inside the hub is left alone.
#[test]
fn the_hub_leaves_a_player_inside_it_alone() {
    let mut game = Game::new();
    walled_hub(&game);
    phase(&game, Phase::Waiting);

    let inside = game.player("inside", Vec3::new(0.0, 66.0, 0.0));
    game.world
        .entity_from_id(inside)
        .set(Position(Vec3::new(0.0, 66.0, 0.0)));

    game.advance(0.05, 1);

    assert_eq!(
        pushed(&game, inside),
        Vec3::ZERO,
        "a player standing in the middle of the hub was pushed"
    );
}

/// The hub never eliminates: a player who somehow gets below its floor is
/// shoved up, not killed. A lobby you can die in is not a lobby.
#[test]
fn the_hub_does_not_eliminate() {
    let mut game = Game::new();
    walled_hub(&game);
    phase(&game, Phase::Waiting);

    let fallen = game.player("fallen", Vec3::new(0.0, 40.0, 0.0));
    game.world
        .entity_from_id(fallen)
        .set(Position(Vec3::new(0.0, 40.0, 0.0)));
    let before = game.world.entity_from_id(fallen).cloned::<&Lives>().0;

    game.advance(0.05, 1);

    assert_eq!(
        game.world.entity_from_id(fallen).cloned::<&Lives>().0,
        before,
        "the hub took a life"
    );
    assert!(
        pushed(&game, fallen).y > 1e-3,
        "a player below the hub floor should be shoved up, not left to fall"
    );
}

/// The arena still eliminates a player who falls below its kill plane, and does
/// it with no phase gate of its own -- the phase only chose the arena's bounds.
#[test]
fn the_arena_eliminates_below_the_kill_plane() {
    let mut game = Game::new();
    phase(&game, Phase::Playing);

    let arena = game.world.cloned::<&Arena>();
    let faller = game.player("faller", Vec3::new(0.0, arena.kill_y - 10.0, 0.0));
    game.world
        .entity_from_id(faller)
        .set(Position(Vec3::new(0.0, arena.kill_y - 10.0, 0.0)));
    let before = game.world.entity_from_id(faller).cloned::<&Lives>().0;

    game.advance(0.05, 1);

    assert!(
        game.world.entity_from_id(faller).cloned::<&Lives>().0 < before,
        "the kill plane did not fire below y={}",
        arena.kill_y
    );
}

/// The results screen enforces neither edge: a player sliding off in the last
/// instant is teleported home, not eliminated a second time.
#[test]
fn nothing_is_enforced_on_the_results_screen() {
    let mut game = Game::new();
    phase(&game, Phase::Ended);

    let arena = game.world.cloned::<&Arena>();
    let slid = game.player("slid", Vec3::new(0.0, arena.kill_y - 10.0, 0.0));
    game.world
        .entity_from_id(slid)
        .set(Position(Vec3::new(0.0, arena.kill_y - 10.0, 0.0)));
    let before = game.world.entity_from_id(slid).cloned::<&Lives>().0;

    game.advance(0.05, 1);

    assert_eq!(
        game.world.entity_from_id(slid).cloned::<&Lives>().0,
        before,
        "the results screen eliminated somebody"
    );
    assert_eq!(
        pushed(&game, slid),
        Vec3::ZERO,
        "the results screen pushed somebody"
    );
}
